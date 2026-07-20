#include "ipc_transport.hpp"

#include "obfuscated_string.hpp"

#include <Windows.h>

#include <cstddef>
#include <cstdint>

namespace nte::equipment
{
	namespace
	{
		constexpr ULONGLONG IPC_CLIENT_IO_TIMEOUT_MS = 1000;

		static_assert(sizeof(NteEquipmentIpcRequest) == NTE_EQUIPMENT_IPC_REQUEST_SIZE);
		static_assert(sizeof(NteEquipmentIpcResponse) == NTE_EQUIPMENT_IPC_RESPONSE_SIZE);

		enum class IpcTransportState
		{
			Closed,
			Listening,
			Reading,
			Ready,
			Writing,
		};

		enum class IpcPollResult
		{
			Error,
			Idle,
			RequestReady,
		};

		HANDLE ipc_pipe = INVALID_HANDLE_VALUE;
		HANDLE ipc_event = nullptr;
		OVERLAPPED ipc_overlapped{};
		IpcTransportState ipc_transport_state = IpcTransportState::Closed;
		ULONGLONG ipc_io_deadline = 0;
		NteEquipmentIpcRequest ipc_request{};
		NteEquipmentIpcResponse ipc_response{};

		bool IsZeroItemId(const NteItemNetId& item)
		{
			return item.slot == 0 && item.serial == 0;
		}

		bool IsZeroPlacement(const NteEquipmentPlacement& placement)
		{
			return IsZeroItemId(placement.equipment) && placement.row == 0 &&
				placement.column == 0;
		}

		bool HasOnlyZeroPlacements(
			const NteEquipmentIpcRequest& request,
			uint32_t first)
		{
			for (uint32_t index = first; index < NTE_EQUIPMENT_MAX_PLACEMENTS; ++index)
			{
				if (!IsZeroPlacement(request.placements[index]))
					return false;
			}
			return true;
		}

		void CloseIpcPipe()
		{
			if (ipc_pipe != INVALID_HANDLE_VALUE)
			{
				CancelIoEx(ipc_pipe, &ipc_overlapped);
				DisconnectNamedPipe(ipc_pipe);
				CloseHandle(ipc_pipe);
			}
			if (ipc_event != nullptr)
				CloseHandle(ipc_event);

			ipc_pipe = INVALID_HANDLE_VALUE;
			ipc_event = nullptr;
			ipc_overlapped = {};
			ipc_transport_state = IpcTransportState::Closed;
			ipc_io_deadline = 0;
			ipc_request = {};
			ipc_response = {};
		}

		void ResetIpcOverlapped()
		{
			ipc_overlapped = {};
			ipc_overlapped.hEvent = ipc_event;
			ResetEvent(ipc_event);
		}

		IpcPollResult BeginIpcRead();

		IpcPollResult BeginIpcConnect()
		{
			ResetIpcOverlapped();
			if (ConnectNamedPipe(ipc_pipe, &ipc_overlapped))
				return BeginIpcRead();

			const DWORD error = GetLastError();
			if (error == ERROR_PIPE_CONNECTED)
				return BeginIpcRead();
			if (error != ERROR_IO_PENDING)
			{
				CloseIpcPipe();
				return IpcPollResult::Error;
			}

			ipc_transport_state = IpcTransportState::Listening;
			return IpcPollResult::Idle;
		}

		IpcPollResult BeginIpcRead()
		{
			ipc_request = {};
			ResetIpcOverlapped();

			DWORD bytes_read = 0;
			if (ReadFile(
				ipc_pipe,
				&ipc_request,
				sizeof(ipc_request),
				&bytes_read,
				&ipc_overlapped))
			{
				if (bytes_read != sizeof(ipc_request))
				{
					CloseIpcPipe();
					return IpcPollResult::Error;
				}
				ipc_transport_state = IpcTransportState::Ready;
				return IpcPollResult::RequestReady;
			}

			const DWORD error = GetLastError();
			if (error != ERROR_IO_PENDING)
			{
				CloseIpcPipe();
				return IpcPollResult::Error;
			}

			ipc_transport_state = IpcTransportState::Reading;
			ipc_io_deadline = GetTickCount64() + IPC_CLIENT_IO_TIMEOUT_MS;
			return IpcPollResult::Idle;
		}

		bool EnsureIpcPipe()
		{
			if (ipc_pipe != INVALID_HANDLE_VALUE)
				return true;

			ipc_event = CreateEventW(nullptr, TRUE, FALSE, nullptr);
			if (ipc_event == nullptr)
				return false;

			const auto pipe_name = NTE_OBFUSCATE_STRING(
				L"\\\\.\\pipe\\nte-equipment-plugin-v3");
			ipc_pipe = CreateNamedPipeW(
				pipe_name.c_str(),
				PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED,
				PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT |
				PIPE_REJECT_REMOTE_CLIENTS,
				1,
				sizeof(NteEquipmentIpcResponse),
				sizeof(NteEquipmentIpcRequest),
				0,
				nullptr);
			if (ipc_pipe == INVALID_HANDLE_VALUE)
			{
				CloseIpcPipe();
				return false;
			}

			return BeginIpcConnect() != IpcPollResult::Error;
		}

		IpcPollResult PollIpcRequest()
		{
			if (!EnsureIpcPipe())
				return IpcPollResult::Error;

			if (ipc_transport_state == IpcTransportState::Ready)
				return IpcPollResult::RequestReady;

			if (ipc_transport_state == IpcTransportState::Closed)
				return BeginIpcConnect();

			if ((ipc_transport_state == IpcTransportState::Reading ||
				ipc_transport_state == IpcTransportState::Writing) &&
				GetTickCount64() >= ipc_io_deadline)
			{
				CloseIpcPipe();
				return IpcPollResult::Idle;
			}
			if (!HasOverlappedIoCompleted(&ipc_overlapped))
				return IpcPollResult::Idle;

			DWORD transferred = 0;
			if (!GetOverlappedResult(
				ipc_pipe, &ipc_overlapped, &transferred, FALSE))
			{
				const DWORD error = GetLastError();
				if (error == ERROR_IO_INCOMPLETE)
					return IpcPollResult::Idle;

				CloseIpcPipe();
				return error == ERROR_BROKEN_PIPE || error == ERROR_NO_DATA
					? IpcPollResult::Idle
					: IpcPollResult::Error;
			}

			switch (ipc_transport_state)
			{
			case IpcTransportState::Listening:
				return BeginIpcRead();
			case IpcTransportState::Reading:
				if (transferred != sizeof(ipc_request))
				{
					CloseIpcPipe();
					return IpcPollResult::Error;
				}
				ipc_transport_state = IpcTransportState::Ready;
				ipc_io_deadline = 0;
				return IpcPollResult::RequestReady;
			case IpcTransportState::Writing:
				if (transferred != sizeof(ipc_response))
				{
					CloseIpcPipe();
					return IpcPollResult::Error;
				}
				DisconnectNamedPipe(ipc_pipe);
				ipc_transport_state = IpcTransportState::Closed;
				ipc_io_deadline = 0;
				return BeginIpcConnect();
			default:
				CloseIpcPipe();
				return IpcPollResult::Error;
			}
		}

		NteEquipmentStatus DispatchIpcRequest(
			const EquipmentContext* context,
			const NteEquipmentIpcRequest& request)
		{
			if (request.magic != NTE_EQUIPMENT_IPC_MAGIC ||
				request.version != NTE_EQUIPMENT_IPC_VERSION ||
				request.request_id == 0 ||
				request.placement_count > NTE_EQUIPMENT_MAX_PLACEMENTS)
				return NTE_EQUIPMENT_STATUS_INVALID_IPC_REQUEST;

			switch (request.operation)
			{
			case NTE_EQUIPMENT_IPC_EQUIP_MODULE:
				if (!IsZeroItemId(request.core) || request.placement_count != 0 ||
					request.state != 0 || !HasOnlyZeroPlacements(request, 0))
					return NTE_EQUIPMENT_STATUS_INVALID_IPC_REQUEST;
				return EquipModule(
					context,
					&request.character,
					&request.equipment,
					request.row,
					request.column);
			case NTE_EQUIPMENT_IPC_EQUIP_CORE:
				if (!IsZeroItemId(request.core) || request.row != 0 ||
					request.column != 0 || request.placement_count != 0 ||
					request.state != 0 || !HasOnlyZeroPlacements(request, 0))
					return NTE_EQUIPMENT_STATUS_INVALID_IPC_REQUEST;
				return EquipCore(
					context, &request.character, &request.equipment);
			case NTE_EQUIPMENT_IPC_UNEQUIP_MODULE:
				if (!IsZeroItemId(request.core) || request.row != 0 ||
					request.column != 0 || request.placement_count != 0 ||
					request.state != 0 || !HasOnlyZeroPlacements(request, 0))
					return NTE_EQUIPMENT_STATUS_INVALID_IPC_REQUEST;
				return UnequipModule(
					context, &request.character, &request.equipment);
			case NTE_EQUIPMENT_IPC_UNEQUIP_CORE:
				if (!IsZeroItemId(request.core) || request.row != 0 ||
					request.column != 0 || request.placement_count != 0 ||
					request.state != 0 || !HasOnlyZeroPlacements(request, 0))
					return NTE_EQUIPMENT_STATUS_INVALID_IPC_REQUEST;
				return UnequipCore(
					context, &request.character, &request.equipment);
			case NTE_EQUIPMENT_IPC_UNEQUIP_ALL:
				if (!IsZeroItemId(request.equipment) || !IsZeroItemId(request.core) ||
					request.row != 0 || request.column != 0 ||
					request.placement_count != 0 || request.state != 0 ||
					!HasOnlyZeroPlacements(request, 0))
					return NTE_EQUIPMENT_STATUS_INVALID_IPC_REQUEST;
				return UnequipAll(context, &request.character);
			case NTE_EQUIPMENT_IPC_EQUIP_ONE_KEY:
				if (!IsZeroItemId(request.equipment) || request.row != 0 ||
					request.column != 0 || request.placement_count == 0 ||
					request.state != 0 ||
					!HasOnlyZeroPlacements(request, request.placement_count))
					return NTE_EQUIPMENT_STATUS_INVALID_IPC_REQUEST;
				return EquipOneKey(
					context,
					&request.character,
					request.placements,
					request.placement_count,
					&request.core);
			case NTE_EQUIPMENT_IPC_MOVE_MODULE_TO_CHARACTER:
				if (!IsZeroItemId(request.core) || request.placement_count != 0 ||
					request.state != 0 || !HasOnlyZeroPlacements(request, 0))
					return NTE_EQUIPMENT_STATUS_INVALID_IPC_REQUEST;
				return MoveModuleToCharacter(
					context,
					&request.character,
					&request.equipment,
					request.row,
					request.column);
			case NTE_EQUIPMENT_IPC_MOVE_CORE_TO_CHARACTER:
				if (!IsZeroItemId(request.core) || request.row != 0 ||
					request.column != 0 || request.placement_count != 0 ||
					request.state != 0 || !HasOnlyZeroPlacements(request, 0))
					return NTE_EQUIPMENT_STATUS_INVALID_IPC_REQUEST;
				return MoveCoreToCharacter(
					context, &request.character, &request.equipment);
			case NTE_EQUIPMENT_IPC_SET_ITEM_DISCARDED:
				if (!IsZeroItemId(request.character) || !IsZeroItemId(request.core) ||
					request.row != 0 || request.column != 0 ||
					request.placement_count != 0 || !HasOnlyZeroPlacements(request, 0))
					return NTE_EQUIPMENT_STATUS_INVALID_IPC_REQUEST;
				return SetItemDiscarded(
					context, &request.equipment, request.state);
			case NTE_EQUIPMENT_IPC_SET_ITEM_LOCKED:
				if (!IsZeroItemId(request.character) || !IsZeroItemId(request.core) ||
					request.row != 0 || request.column != 0 ||
					request.placement_count != 0 || !HasOnlyZeroPlacements(request, 0))
					return NTE_EQUIPMENT_STATUS_INVALID_IPC_REQUEST;
				return SetItemLocked(context, &request.equipment, request.state);
			default:
				return NTE_EQUIPMENT_STATUS_INVALID_IPC_REQUEST;
			}
		}

		IpcPumpResult CompleteIpcRequest(
			const EquipmentContext* context)
		{
			const NteEquipmentStatus status = DispatchIpcRequest(context, ipc_request);
			ipc_response = {
				NTE_EQUIPMENT_IPC_MAGIC,
				NTE_EQUIPMENT_IPC_VERSION,
				0,
				ipc_request.request_id,
				static_cast<uint32_t>(status),
				0,
			};

			ResetIpcOverlapped();
			DWORD bytes_written = 0;
			if (WriteFile(
				ipc_pipe,
				&ipc_response,
				sizeof(ipc_response),
				&bytes_written,
				&ipc_overlapped))
			{
				if (bytes_written != sizeof(ipc_response))
				{
					CloseIpcPipe();
					return IpcPumpResult::Error;
				}

				DisconnectNamedPipe(ipc_pipe);
				ipc_transport_state = IpcTransportState::Closed;
				ipc_io_deadline = 0;
				BeginIpcConnect();
				return IpcPumpResult::Processed;
			}

			if (GetLastError() != ERROR_IO_PENDING)
			{
				CloseIpcPipe();
				return IpcPumpResult::Error;
			}

			ipc_transport_state = IpcTransportState::Writing;
			ipc_io_deadline = GetTickCount64() + IPC_CLIENT_IO_TIMEOUT_MS;
			return IpcPumpResult::Processed;
		}

	} // namespace

	IpcPumpResult PumpLiveIpc(
		void* user_data,
		PlayerStateResolver resolve_player_state)
	{
		const IpcPollResult poll_result = PollIpcRequest();
		if (poll_result == IpcPollResult::Error)
			return IpcPumpResult::Error;
		if (poll_result == IpcPollResult::Idle)
			return IpcPumpResult::Idle;
		if (resolve_player_state == nullptr)
			return IpcPumpResult::Error;

		const EquipmentContext context{ resolve_player_state(user_data) };
		return CompleteIpcRequest(&context);
	}

	void CloseIpc()
	{
		CloseIpcPipe();
	}
} // namespace nte::equipment
