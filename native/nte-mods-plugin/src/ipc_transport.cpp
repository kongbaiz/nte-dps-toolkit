#include "ipc_transport.hpp"

#include "mod_runtime.hpp"
#include "obfuscated_string.hpp"

#include <Windows.h>

#include <cstddef>
#include <cstdint>

namespace nte::mods
{
	namespace
	{
		constexpr ULONGLONG IPC_CLIENT_IO_TIMEOUT_MS = 1000;

		static_assert(sizeof(NteModsIpcRequest) == NTE_MODS_IPC_REQUEST_SIZE);
		static_assert(sizeof(NteModsIpcResponse) == NTE_MODS_IPC_RESPONSE_SIZE);

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
		NteModsIpcRequest ipc_request{};
		NteModsIpcResponse ipc_response{};

		class PipeSecurityAttributes
		{
		public:
			PipeSecurityAttributes() = default;
			PipeSecurityAttributes(const PipeSecurityAttributes&) = delete;
			PipeSecurityAttributes& operator=(const PipeSecurityAttributes&) = delete;

			~PipeSecurityAttributes()
			{
				if (descriptor_ != nullptr)
					LocalFree(descriptor_);
				if (advapi_ != nullptr)
					FreeLibrary(advapi_);
			}

			bool Initialize()
			{
				const auto library_name =
					NTE_OBFUSCATE_STRING(L"advapi32.dll");
				advapi_ = LoadLibraryW(library_name.c_str());
				if (advapi_ == nullptr)
					return false;

				using ConvertSecurityDescriptor =
					BOOL(WINAPI*)(LPCWSTR, DWORD, PSECURITY_DESCRIPTOR*, PULONG);
				const auto function_name = NTE_OBFUSCATE_STRING(
					"ConvertStringSecurityDescriptorToSecurityDescriptorW");
				const auto convert = reinterpret_cast<ConvertSecurityDescriptor>(
					GetProcAddress(advapi_, function_name.c_str()));
				if (convert == nullptr)
					return false;

				// The game runs at high integrity while the desktop client normally
				// runs at medium integrity. Keep the pipe local and grant access only
				// to system, administrators, and the interactive desktop session.
				const auto descriptor = NTE_OBFUSCATE_STRING(
					L"D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GA;;;IU)"
					L"S:(ML;;NW;;;ME)");
				if (!convert(
					descriptor.c_str(), 1, &descriptor_, nullptr))
					return false;

				attributes_ = {
					sizeof(SECURITY_ATTRIBUTES),
					descriptor_,
					FALSE,
				};
				return true;
			}

			SECURITY_ATTRIBUTES* Get()
			{
				return &attributes_;
			}

		private:
			HMODULE advapi_ = nullptr;
			PSECURITY_DESCRIPTOR descriptor_ = nullptr;
			SECURITY_ATTRIBUTES attributes_{};
		};

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
			const NteModsIpcRequest& request,
			uint32_t first)
		{
			for (uint32_t index = first; index < NTE_EQUIPMENT_MAX_PLACEMENTS; ++index)
			{
				if (!IsZeroPlacement(request.placements[index]))
					return false;
			}
			return true;
		}

		bool IsEmptyQueryRequest(const NteModsIpcRequest& request)
		{
			return IsZeroItemId(request.character) &&
				IsZeroItemId(request.equipment) &&
				IsZeroItemId(request.core) &&
				request.row == 0 && request.column == 0 &&
				request.placement_count == 0 && request.state == 0 &&
				HasOnlyZeroPlacements(request, 0);
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

			PipeSecurityAttributes security;
			if (!security.Initialize())
				return false;

			ipc_event = CreateEventW(nullptr, TRUE, FALSE, nullptr);
			if (ipc_event == nullptr)
				return false;

			const auto pipe_name = NTE_OBFUSCATE_STRING(
				NTE_MODS_PIPE_NAME);
			ipc_pipe = CreateNamedPipeW(
				pipe_name.c_str(),
				PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED,
				PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT |
				PIPE_REJECT_REMOTE_CLIENTS,
				1,
				sizeof(NteModsIpcResponse),
				sizeof(NteModsIpcRequest),
				0,
				security.Get());
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

		NteModsStatus InvokeIpcKernelServiceImpl(
			IpcKernelService service,
			const PluginContext* context,
			const NteModsIpcRequest& request,
			NteModsIpcResponse& response)
		{
			switch (service)
			{
			case IpcKernelService::QueryCombatClockTransitions:
				if (!IsEmptyQueryRequest(request))
					return NTE_MODS_STATUS_INVALID_IPC_REQUEST;
				response.record_count =
					CopyCombatClockTransitions(
						response.payload.combat_clock_transitions,
						NTE_COMBAT_CLOCK_HISTORY_SIZE);
				return NTE_MODS_STATUS_DRY_RUN_OK;
			case IpcKernelService::QueryModEvents:
				if (!IsEmptyQueryRequest(request))
					return NTE_MODS_STATUS_INVALID_IPC_REQUEST;
				response.record_count = runtime::CopyModEvents(
					response.payload.mod_events,
					NTE_MOD_EVENT_HISTORY_SIZE);
				return NTE_MODS_STATUS_DRY_RUN_OK;
			case IpcKernelService::QueryModLogs:
				if (!IsEmptyQueryRequest(request))
					return NTE_MODS_STATUS_INVALID_IPC_REQUEST;
				response.record_count = runtime::CopyModLogs(
					response.payload.mod_logs,
					NTE_MOD_LOG_HISTORY_SIZE);
				return NTE_MODS_STATUS_DRY_RUN_OK;
			case IpcKernelService::EquipModule:
				if (!IsZeroItemId(request.core) || request.placement_count != 0 ||
					request.state != 0 || !HasOnlyZeroPlacements(request, 0))
					return NTE_MODS_STATUS_INVALID_IPC_REQUEST;
				return EquipModule(
					context,
					&request.character,
					&request.equipment,
					request.row,
					request.column);
			case IpcKernelService::EquipCore:
				if (!IsZeroItemId(request.core) || request.row != 0 ||
					request.column != 0 || request.placement_count != 0 ||
					request.state != 0 || !HasOnlyZeroPlacements(request, 0))
					return NTE_MODS_STATUS_INVALID_IPC_REQUEST;
				return EquipCore(
					context, &request.character, &request.equipment);
			case IpcKernelService::UnequipModule:
				if (!IsZeroItemId(request.core) || request.row != 0 ||
					request.column != 0 || request.placement_count != 0 ||
					request.state != 0 || !HasOnlyZeroPlacements(request, 0))
					return NTE_MODS_STATUS_INVALID_IPC_REQUEST;
				return UnequipModule(
					context, &request.character, &request.equipment);
			case IpcKernelService::UnequipCore:
				if (!IsZeroItemId(request.core) || request.row != 0 ||
					request.column != 0 || request.placement_count != 0 ||
					request.state != 0 || !HasOnlyZeroPlacements(request, 0))
					return NTE_MODS_STATUS_INVALID_IPC_REQUEST;
				return UnequipCore(
					context, &request.character, &request.equipment);
			case IpcKernelService::UnequipAll:
				if (!IsZeroItemId(request.equipment) || !IsZeroItemId(request.core) ||
					request.row != 0 || request.column != 0 ||
					request.placement_count != 0 || request.state != 0 ||
					!HasOnlyZeroPlacements(request, 0))
					return NTE_MODS_STATUS_INVALID_IPC_REQUEST;
				return UnequipAll(context, &request.character);
			case IpcKernelService::EquipOneKey:
				if (!IsZeroItemId(request.equipment) || request.row != 0 ||
					request.column != 0 || request.placement_count == 0 ||
					request.state != 0 ||
					!HasOnlyZeroPlacements(request, request.placement_count))
					return NTE_MODS_STATUS_INVALID_IPC_REQUEST;
				return EquipOneKey(
					context,
					&request.character,
					request.placements,
					request.placement_count,
					&request.core);
			case IpcKernelService::MoveModuleToCharacter:
				if (!IsZeroItemId(request.core) || request.placement_count != 0 ||
					request.state != 0 || !HasOnlyZeroPlacements(request, 0))
					return NTE_MODS_STATUS_INVALID_IPC_REQUEST;
				return MoveModuleToCharacter(
					context,
					&request.character,
					&request.equipment,
					request.row,
					request.column);
			case IpcKernelService::MoveCoreToCharacter:
				if (!IsZeroItemId(request.core) || request.row != 0 ||
					request.column != 0 || request.placement_count != 0 ||
					request.state != 0 || !HasOnlyZeroPlacements(request, 0))
					return NTE_MODS_STATUS_INVALID_IPC_REQUEST;
				return MoveCoreToCharacter(
					context, &request.character, &request.equipment);
			case IpcKernelService::SetItemDiscarded:
				if (!IsZeroItemId(request.character) || !IsZeroItemId(request.core) ||
					request.row != 0 || request.column != 0 ||
					request.placement_count != 0 || !HasOnlyZeroPlacements(request, 0))
					return NTE_MODS_STATUS_INVALID_IPC_REQUEST;
				return SetItemDiscarded(
					context, &request.equipment, request.state);
			case IpcKernelService::SetItemLocked:
				if (!IsZeroItemId(request.character) || !IsZeroItemId(request.core) ||
					request.row != 0 || request.column != 0 ||
					request.placement_count != 0 || !HasOnlyZeroPlacements(request, 0))
					return NTE_MODS_STATUS_INVALID_IPC_REQUEST;
				return SetItemLocked(context, &request.equipment, request.state);
			}
			return NTE_MODS_STATUS_INVALID_IPC_REQUEST;
		}

		NteModsStatus DispatchIpcRequest(
			const PluginContext* context,
			const NteModsIpcRequest& request,
			NteModsIpcResponse& response)
		{
			if (request.magic != NTE_MODS_IPC_MAGIC ||
				request.version != NTE_MODS_IPC_VERSION ||
				request.request_id == 0 ||
				request.operation < NTE_MODS_IPC_EQUIP_MODULE ||
				request.operation > NTE_MODS_IPC_QUERY_MOD_LOGS ||
				request.placement_count > NTE_EQUIPMENT_MAX_PLACEMENTS)
				return NTE_MODS_STATUS_INVALID_IPC_REQUEST;
			if (request.operation == NTE_MODS_IPC_QUERY_MOD_LOGS)
			{
				return InvokeIpcKernelServiceImpl(
					IpcKernelService::QueryModLogs,
					context,
					request,
					response);
			}
			return runtime::DispatchIpcRequestPrograms(
				context,
				request,
				response);
		}

		IpcPumpResult CompleteIpcRequest(const PluginContext* context)
		{
			ipc_response = {};
			ipc_response.magic = NTE_MODS_IPC_MAGIC;
			ipc_response.version = NTE_MODS_IPC_VERSION;
			ipc_response.request_id = ipc_request.request_id;
			const NteModsStatus status = DispatchIpcRequest(
				context, ipc_request, ipc_response);
			ipc_response.status = static_cast<uint32_t>(status);

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

	NteModsStatus InvokeIpcKernelService(
		IpcKernelService service,
		const PluginContext* context,
		const NteModsIpcRequest& request,
		NteModsIpcResponse& response)
	{
		return InvokeIpcKernelServiceImpl(
			service,
			context,
			request,
			response);
	}

	IpcPumpResult PumpLiveIpc(
		const PluginContext* context)
	{
		if (context == nullptr)
			return IpcPumpResult::Error;

		const IpcPollResult poll_result = PollIpcRequest();
		if (poll_result == IpcPollResult::Error)
			return IpcPumpResult::Error;
		if (poll_result == IpcPollResult::Idle)
			return IpcPumpResult::Idle;
		return CompleteIpcRequest(context);
	}

	void CloseIpc()
	{
		CloseIpcPipe();
	}
} // namespace nte::mods
