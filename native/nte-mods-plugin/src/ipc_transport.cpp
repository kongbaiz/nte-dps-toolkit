#include "ipc_transport.hpp"

#include "ipc_transport_policy.hpp"
#include "mod_runtime.hpp"
#include "obfuscated_string.hpp"

#include <Aclapi.h>
#include <Windows.h>

#include <array>
#include <cstddef>
#include <cstdint>

namespace nte::mods
{
	namespace
	{
		constexpr ULONGLONG IPC_CLIENT_IO_TIMEOUT_MS = 1000;
		constexpr DWORD IPC_PIPE_CLIENT_ACCESS =
			FILE_READ_DATA | FILE_WRITE_DATA | SYNCHRONIZE;
		constexpr DWORD IPC_PRESENCE_OWNER_ACCESS =
			EVENT_MODIFY_STATE | SYNCHRONIZE | READ_CONTROL;
		// Later clients may wait and inspect owner/DACL identity, but cannot signal
		// or reset the published event.
		constexpr DWORD IPC_PRESENCE_CLIENT_ACCESS =
			SYNCHRONIZE | READ_CONTROL;

		static_assert(sizeof(NteModsIpcRequest) == NTE_MODS_IPC_REQUEST_SIZE);
		static_assert(sizeof(NteModsIpcResponse) == NTE_MODS_IPC_RESPONSE_SIZE);
		static_assert(
			sizeof(NteModsIpcDeliveryAck) == NTE_MODS_IPC_DELIVERY_ACK_SIZE);

		enum class IpcTransportState
		{
			Closed,
			Listening,
			Reading,
			Ready,
			Writing,
			AwaitingClientAck,
			Closing,
		};

		enum class IpcPollResult
		{
			Error,
			Idle,
			RequestReady,
		};

		struct TokenIdentity
		{
			std::array<uint8_t, SECURITY_MAX_SID_SIZE> user_sid{};
			std::array<uint8_t, SECURITY_MAX_SID_SIZE> logon_sid{};
			DWORD session_id = 0;
			bool valid = false;

			PSID UserSid()
			{
				return user_sid.data();
			}
			PSID UserSid() const
			{
				return const_cast<uint8_t*>(user_sid.data());
			}

			PSID LogonSid()
			{
				return logon_sid.data();
			}
			PSID LogonSid() const
			{
				return const_cast<uint8_t*>(logon_sid.data());
			}
		};

		HANDLE ipc_pipe = INVALID_HANDLE_VALUE;
		HANDLE ipc_event = nullptr;
		HANDLE runtime_presence_event = nullptr;
		OVERLAPPED ipc_overlapped{};
		SRWLOCK ipc_transport_lock = SRWLOCK_INIT;
		SRWLOCK runtime_presence_lock = SRWLOCK_INIT;
		IpcTransportState ipc_transport_state = IpcTransportState::Closed;
		ULONGLONG ipc_io_deadline = 0;
		uint64_t ipc_generation = 0;
		ipc::OperationEpoch ipc_operation{};
		TokenIdentity ipc_server_identity{};
		bool ipc_stopping = false;
		NteModsIpcRequest ipc_request{};
		NteModsIpcResponse ipc_response{};
		NteModsIpcDeliveryAck ipc_delivery_ack{};

		class IpcTransportGuard
		{
		public:
			IpcTransportGuard()
			{
				AcquireSRWLockExclusive(&ipc_transport_lock);
			}

			IpcTransportGuard(const IpcTransportGuard&) = delete;
			IpcTransportGuard& operator=(const IpcTransportGuard&) = delete;

			~IpcTransportGuard()
			{
				ReleaseSRWLockExclusive(&ipc_transport_lock);
			}
		};

		class IpcTransportTryGuard
		{
		public:
			IpcTransportTryGuard()
				: acquired_(TryAcquireSRWLockExclusive(&ipc_transport_lock) != FALSE)
			{
			}

			IpcTransportTryGuard(const IpcTransportTryGuard&) = delete;
			IpcTransportTryGuard& operator=(const IpcTransportTryGuard&) = delete;

			~IpcTransportTryGuard()
			{
				if (acquired_)
					ReleaseSRWLockExclusive(&ipc_transport_lock);
			}

			bool Acquired() const
			{
				return acquired_;
			}

		private:
			bool acquired_;
		};

		class RuntimePresenceGuard
		{
		public:
			RuntimePresenceGuard()
			{
				AcquireSRWLockExclusive(&runtime_presence_lock);
			}

			RuntimePresenceGuard(const RuntimePresenceGuard&) = delete;
			RuntimePresenceGuard& operator=(const RuntimePresenceGuard&) = delete;

			~RuntimePresenceGuard()
			{
				ReleaseSRWLockExclusive(&runtime_presence_lock);
			}
		};

		class OwnedHandle
		{
		public:
			explicit OwnedHandle(HANDLE value = nullptr)
				: value_(value)
			{
			}

			OwnedHandle(const OwnedHandle&) = delete;
			OwnedHandle& operator=(const OwnedHandle&) = delete;

			~OwnedHandle()
			{
				if (value_ != nullptr && value_ != INVALID_HANDLE_VALUE)
					CloseHandle(value_);
			}

			HANDLE Get() const
			{
				return value_;
			}

		private:
			HANDLE value_;
		};

		class LocalAllocation
		{
		public:
			LocalAllocation() = default;
			LocalAllocation(const LocalAllocation&) = delete;
			LocalAllocation& operator=(const LocalAllocation&) = delete;

			~LocalAllocation()
			{
				if (value_ != nullptr)
					LocalFree(value_);
			}

			bool Allocate(SIZE_T size)
			{
				if (value_ != nullptr || size == 0)
					return false;
				value_ = LocalAlloc(LPTR, size);
				return value_ != nullptr;
			}

			void* Get() const
			{
				return value_;
			}

		private:
			HLOCAL value_ = nullptr;
		};

		bool QueryTokenIdentity(HANDLE token, TokenIdentity& identity)
		{
			identity = {};

			DWORD required = 0;
			GetTokenInformation(token, TokenUser, nullptr, 0, &required);
			if (required < sizeof(TOKEN_USER) ||
				GetLastError() != ERROR_INSUFFICIENT_BUFFER)
				return false;
			LocalAllocation user_buffer;
			if (!user_buffer.Allocate(required) ||
				!GetTokenInformation(
					token,
					TokenUser,
					user_buffer.Get(),
					required,
					&required))
				return false;
			const auto* token_user = static_cast<const TOKEN_USER*>(
				user_buffer.Get());
			if (!IsValidSid(token_user->User.Sid) ||
				GetLengthSid(token_user->User.Sid) > identity.user_sid.size() ||
				!CopySid(
					static_cast<DWORD>(identity.user_sid.size()),
					identity.UserSid(),
					token_user->User.Sid))
				return false;

			DWORD returned = 0;
			if (!GetTokenInformation(
					token,
					TokenSessionId,
					&identity.session_id,
					sizeof(identity.session_id),
					&returned) ||
				returned != sizeof(identity.session_id))
				return false;

			required = 0;
			GetTokenInformation(token, TokenGroups, nullptr, 0, &required);
			if (required < sizeof(TOKEN_GROUPS) ||
				GetLastError() != ERROR_INSUFFICIENT_BUFFER)
				return false;
			LocalAllocation groups_buffer;
			if (!groups_buffer.Allocate(required) ||
				!GetTokenInformation(
					token,
					TokenGroups,
					groups_buffer.Get(),
					required,
					&required))
				return false;
			const auto* token_groups = static_cast<const TOKEN_GROUPS*>(
				groups_buffer.Get());
			for (DWORD index = 0; index < token_groups->GroupCount; ++index)
			{
				const SID_AND_ATTRIBUTES& group = token_groups->Groups[index];
				if ((group.Attributes & SE_GROUP_LOGON_ID) != SE_GROUP_LOGON_ID ||
					!IsValidSid(group.Sid) ||
					GetLengthSid(group.Sid) > identity.logon_sid.size())
					continue;
				if (!CopySid(
						static_cast<DWORD>(identity.logon_sid.size()),
						identity.LogonSid(),
						group.Sid))
					return false;
				identity.valid = true;
				return true;
			}
			return false;
		}

		bool QueryCurrentTokenIdentity(TokenIdentity& identity)
		{
			HANDLE token_value = nullptr;
			if (!OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &token_value))
				return false;
			const OwnedHandle token(token_value);
			return QueryTokenIdentity(token.Get(), identity);
		}

		class LocalIpcSecurityAttributes
		{
		public:
			LocalIpcSecurityAttributes() = default;
			LocalIpcSecurityAttributes(const LocalIpcSecurityAttributes&) = delete;
			LocalIpcSecurityAttributes& operator=(const LocalIpcSecurityAttributes&) = delete;

			~LocalIpcSecurityAttributes()
			{
				if (dacl_ != nullptr)
					LocalFree(dacl_);
				if (sacl_ != nullptr)
					LocalFree(sacl_);
			}

			bool Initialize(
				DWORD client_access,
				WELL_KNOWN_SID_TYPE mandatory_label,
				TokenIdentity* identity_out)
			{
				if (!QueryCurrentTokenIdentity(identity_))
					return false;

				const DWORD logon_sid_length = GetLengthSid(identity_.LogonSid());
				const SIZE_T dacl_size = sizeof(ACL) +
					sizeof(ACCESS_ALLOWED_ACE) - sizeof(DWORD) + logon_sid_length;
				dacl_ = static_cast<PACL>(LocalAlloc(LPTR, dacl_size));
				if (dacl_ == nullptr ||
					!InitializeAcl(dacl_, static_cast<DWORD>(dacl_size), ACL_REVISION))
					return false;
				if (!AddAccessAllowedAceEx(
						dacl_,
						ACL_REVISION,
						0,
						client_access,
						identity_.LogonSid()))
					return false;

				DWORD mandatory_sid_size =
					static_cast<DWORD>(mandatory_sid_.size());
				if (!CreateWellKnownSid(
						mandatory_label,
						nullptr,
						mandatory_sid_.data(),
						&mandatory_sid_size))
					return false;
				const SIZE_T sacl_size = sizeof(ACL) +
					sizeof(SYSTEM_MANDATORY_LABEL_ACE) - sizeof(DWORD) +
					GetLengthSid(mandatory_sid_.data());
				sacl_ = static_cast<PACL>(LocalAlloc(LPTR, sacl_size));
				if (sacl_ == nullptr ||
					!InitializeAcl(sacl_, static_cast<DWORD>(sacl_size), ACL_REVISION) ||
					!AddMandatoryAce(
						sacl_,
						ACL_REVISION,
						0,
						SYSTEM_MANDATORY_LABEL_NO_WRITE_UP,
						mandatory_sid_.data()) ||
					!InitializeSecurityDescriptor(
						&descriptor_, SECURITY_DESCRIPTOR_REVISION) ||
					!SetSecurityDescriptorOwner(
						&descriptor_, identity_.UserSid(), FALSE) ||
					!SetSecurityDescriptorDacl(
						&descriptor_, TRUE, dacl_, FALSE) ||
					!SetSecurityDescriptorSacl(
						&descriptor_, TRUE, sacl_, FALSE))
					return false;

				attributes_ = {
					sizeof(SECURITY_ATTRIBUTES),
					&descriptor_,
					FALSE,
				};
				if (identity_out != nullptr)
					*identity_out = identity_;
				return true;
			}

			SECURITY_ATTRIBUTES* Get()
			{
				return &attributes_;
			}

		private:
			TokenIdentity identity_{};
			SECURITY_DESCRIPTOR descriptor_{};
			PACL dacl_ = nullptr;
			PACL sacl_ = nullptr;
			std::array<uint8_t, SECURITY_MAX_SID_SIZE> mandatory_sid_{};
			SECURITY_ATTRIBUTES attributes_{};
		};

		bool TokenIdentitiesMatch(
			const TokenIdentity& expected,
			const TokenIdentity& actual)
		{
			return expected.valid && actual.valid &&
				expected.session_id == actual.session_id &&
				EqualSid(expected.UserSid(), actual.UserSid()) != FALSE &&
				EqualSid(expected.LogonSid(), actual.LogonSid()) != FALSE;
		}

		bool ValidateIpcClient()
		{
			if (!ipc_server_identity.valid || ipc_pipe == INVALID_HANDLE_VALUE)
				return false;
			ULONG client_process_id = 0;
			if (!GetNamedPipeClientProcessId(ipc_pipe, &client_process_id) ||
				client_process_id == 0)
				return false;

			const OwnedHandle process(OpenProcess(
				PROCESS_QUERY_LIMITED_INFORMATION,
				FALSE,
				client_process_id));
			if (process.Get() == nullptr)
				return false;
			HANDLE token_value = nullptr;
			if (!OpenProcessToken(process.Get(), TOKEN_QUERY, &token_value))
				return false;
			const OwnedHandle token(token_value);
			TokenIdentity client_identity{};
			return QueryTokenIdentity(token.Get(), client_identity) &&
				TokenIdentitiesMatch(ipc_server_identity, client_identity);
		}

		bool ValidatePresenceEventSecurity(
			HANDLE event,
			const TokenIdentity& expected)
		{
			PSID owner = nullptr;
			PACL dacl = nullptr;
			PSECURITY_DESCRIPTOR security_descriptor = nullptr;
			const DWORD status = GetSecurityInfo(
				event,
				SE_KERNEL_OBJECT,
				OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
				&owner,
				nullptr,
				&dacl,
				nullptr,
				&security_descriptor);
			if (status != ERROR_SUCCESS || security_descriptor == nullptr)
			{
				if (security_descriptor != nullptr)
					LocalFree(security_descriptor);
				return false;
			}

			bool valid = false;
			ACL_SIZE_INFORMATION acl_information{};
			if (expected.valid && owner != nullptr && dacl != nullptr &&
				EqualSid(owner, expected.UserSid()) != FALSE &&
				GetAclInformation(
					dacl,
					&acl_information,
					sizeof(acl_information),
					AclSizeInformation) &&
				acl_information.AceCount == 1)
			{
				void* ace_value = nullptr;
				if (GetAce(dacl, 0, &ace_value) && ace_value != nullptr)
				{
					const auto* ace = static_cast<const ACCESS_ALLOWED_ACE*>(
						ace_value);
					const PSID ace_sid = const_cast<DWORD*>(&ace->SidStart);
					valid = ace->Header.AceType == ACCESS_ALLOWED_ACE_TYPE &&
						ace->Header.AceFlags == 0 &&
						ace->Mask == IPC_PRESENCE_CLIENT_ACCESS &&
						IsValidSid(ace_sid) &&
						EqualSid(ace_sid, expected.LogonSid()) != FALSE;
				}
			}
			LocalFree(security_descriptor);
			return valid;
		}

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

		bool DrainIpcOperation()
		{
			if (!ipc_operation.IsPending())
				return true;
			if (ipc_pipe == INVALID_HANDLE_VALUE ||
				!ipc_operation.BelongsTo(ipc_generation))
				return false;

			// CancelIoEx only requests cancellation. The OVERLAPPED, event, pipe,
			// and request/response buffers remain owned by this generation until
			// GetOverlappedResult observes its terminal completion status.
			CancelIoEx(ipc_pipe, &ipc_overlapped);
			if (ipc_event == nullptr ||
				WaitForSingleObject(
					ipc_event, IPC_CLIENT_IO_TIMEOUT_MS) != WAIT_OBJECT_0)
				return false;
			DWORD transferred = 0;
			if (!GetOverlappedResult(
					ipc_pipe, &ipc_overlapped, &transferred, FALSE))
			{
				const DWORD error = GetLastError();
				if (error == ERROR_IO_INCOMPLETE)
					return false;
			}
			return ipc_operation.Complete(ipc_generation);
		}

		bool CloseIpcPipe()
		{
			if (ipc_pipe != INVALID_HANDLE_VALUE)
			{
				if (ipc_operation.IsPending())
				{
					// A timed-out drain retains this generation and can only resume
					// through PollIpcClose; it must never become request-ready later.
					ipc_transport_state = IpcTransportState::Closing;
					ipc_io_deadline = 0;
				}
				if (!DrainIpcOperation())
					return false;
				DisconnectNamedPipe(ipc_pipe);
				if (!CloseHandle(ipc_pipe))
					return false;
				ipc_pipe = INVALID_HANDLE_VALUE;
			}
			else if (!ipc_operation.CanReuse())
			{
				return false;
			}
			if (ipc_event != nullptr)
			{
				if (!CloseHandle(ipc_event))
					return false;
				ipc_event = nullptr;
			}

			ipc_overlapped = {};
			ipc_transport_state = IpcTransportState::Closed;
			ipc_io_deadline = 0;
			ipc_request = {};
			ipc_response = {};
			ipc_delivery_ack = {};
			ipc_server_identity = {};
			return true;
		}

		IpcPollResult BeginIpcClose()
		{
			if (!ipc_operation.IsPending())
				return CloseIpcPipe()
					? IpcPollResult::Idle
					: IpcPollResult::Error;
			if (ipc_pipe == INVALID_HANDLE_VALUE ||
				!ipc_operation.BelongsTo(ipc_generation))
				return IpcPollResult::Error;

			// The viewport thread never waits for cancellation. It remains the
			// operation owner and consumes the terminal completion on a later pump.
			CancelIoEx(ipc_pipe, &ipc_overlapped);
			ipc_transport_state = IpcTransportState::Closing;
			ipc_io_deadline = 0;
			return IpcPollResult::Idle;
		}

		IpcPollResult PollIpcClose()
		{
			if (!ipc_operation.IsPending())
				return CloseIpcPipe()
					? IpcPollResult::Idle
					: IpcPollResult::Error;
			if (!ipc_operation.BelongsTo(ipc_generation))
				return IpcPollResult::Error;
			if (!HasOverlappedIoCompleted(&ipc_overlapped))
			{
				CancelIoEx(ipc_pipe, &ipc_overlapped);
				return IpcPollResult::Idle;
			}

			DWORD transferred = 0;
			if (!GetOverlappedResult(
					ipc_pipe, &ipc_overlapped, &transferred, FALSE) &&
				GetLastError() == ERROR_IO_INCOMPLETE)
				return IpcPollResult::Idle;
			if (!ipc_operation.Complete(ipc_generation))
				return IpcPollResult::Error;
			return CloseIpcPipe()
				? IpcPollResult::Idle
				: IpcPollResult::Error;
		}

		bool ResetIpcOverlapped()
		{
			if (ipc_event == nullptr || !ipc_operation.CanReuse())
				return false;
			ipc_overlapped = {};
			ipc_overlapped.hEvent = ipc_event;
			return ResetEvent(ipc_event) != FALSE;
		}

		IpcPollResult BeginIpcConnect();
		IpcPollResult BeginIpcRead();

		IpcPollResult ReconnectIpcPipe()
		{
			if (ipc_pipe == INVALID_HANDLE_VALUE || ipc_operation.IsPending())
				return IpcPollResult::Error;
			if (!DisconnectNamedPipe(ipc_pipe))
			{
				const DWORD error = GetLastError();
				if (error != ERROR_PIPE_NOT_CONNECTED && error != ERROR_NO_DATA)
				{
					CloseIpcPipe();
					return IpcPollResult::Error;
				}
			}
			ipc_transport_state = IpcTransportState::Closed;
			ipc_io_deadline = 0;
			ipc_delivery_ack = {};
			return BeginIpcConnect();
		}

		bool IsExpectedDeliveryAck()
		{
			return ipc_delivery_ack.magic == NTE_MODS_IPC_DELIVERY_ACK_MAGIC &&
				ipc_delivery_ack.version == NTE_MODS_IPC_VERSION &&
				ipc_delivery_ack.reserved == 0 &&
				ipc_delivery_ack.request_id == ipc_response.request_id;
		}

		IpcPollResult BeginIpcClientAck()
		{
			ipc_delivery_ack = {};
			if (!ResetIpcOverlapped() ||
				!ipc_operation.Begin(ipc_generation))
			{
				CloseIpcPipe();
				return IpcPollResult::Error;
			}

			DWORD bytes_read = 0;
			if (ReadFile(
				ipc_pipe,
				&ipc_delivery_ack,
				sizeof(ipc_delivery_ack),
				&bytes_read,
				&ipc_overlapped))
			{
				if (!ipc_operation.Complete(ipc_generation) ||
					bytes_read != sizeof(ipc_delivery_ack) ||
					!IsExpectedDeliveryAck())
				{
					CloseIpcPipe();
					return IpcPollResult::Error;
				}
				return ReconnectIpcPipe();
			}

			const DWORD error = GetLastError();
			if (error == ERROR_IO_PENDING)
			{
				ipc_transport_state = IpcTransportState::AwaitingClientAck;
				ipc_io_deadline = GetTickCount64() + IPC_CLIENT_IO_TIMEOUT_MS;
				return IpcPollResult::Idle;
			}
			if (!ipc_operation.Complete(ipc_generation))
				return IpcPollResult::Error;
			if (error == ERROR_BROKEN_PIPE || error == ERROR_NO_DATA)
				return ReconnectIpcPipe();

			CloseIpcPipe();
			return IpcPollResult::Error;
		}

		IpcPollResult BeginIpcConnect()
		{
			if (!ResetIpcOverlapped() ||
				!ipc_operation.Begin(ipc_generation))
				return IpcPollResult::Error;
			if (ConnectNamedPipe(ipc_pipe, &ipc_overlapped))
			{
				ipc_operation.Complete(ipc_generation);
				return BeginIpcRead();
			}

			const DWORD error = GetLastError();
			if (error == ERROR_PIPE_CONNECTED)
			{
				ipc_operation.Complete(ipc_generation);
				return BeginIpcRead();
			}
			if (error != ERROR_IO_PENDING)
			{
				ipc_operation.Complete(ipc_generation);
				CloseIpcPipe();
				return IpcPollResult::Error;
			}

			ipc_transport_state = IpcTransportState::Listening;
			return IpcPollResult::Idle;
		}

		IpcPollResult BeginIpcRead()
		{
			if (!ValidateIpcClient())
			{
				CloseIpcPipe();
				return IpcPollResult::Error;
			}
			ipc_request = {};
			if (!ResetIpcOverlapped() ||
				!ipc_operation.Begin(ipc_generation))
				return IpcPollResult::Error;

			DWORD bytes_read = 0;
			if (ReadFile(
				ipc_pipe,
				&ipc_request,
				sizeof(ipc_request),
				&bytes_read,
				&ipc_overlapped))
			{
				ipc_operation.Complete(ipc_generation);
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
				ipc_operation.Complete(ipc_generation);
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

			LocalIpcSecurityAttributes security;
			TokenIdentity server_identity{};
			if (!security.Initialize(
					IPC_PIPE_CLIENT_ACCESS,
					WinMediumLabelSid,
					&server_identity))
				return false;

			ipc_event = CreateEventW(nullptr, TRUE, FALSE, nullptr);
			if (ipc_event == nullptr)
				return false;

			const auto pipe_name = NTE_OBFUSCATE_STRING(
				NTE_MODS_PIPE_NAME);
			ipc_pipe = CreateNamedPipeW(
				pipe_name.c_str(),
				PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED |
					FILE_FLAG_FIRST_PIPE_INSTANCE,
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
			ipc_server_identity = server_identity;
			++ipc_generation;
			if (ipc_generation == 0)
				++ipc_generation;

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
			if (ipc_transport_state == IpcTransportState::Closing)
				return PollIpcClose();
			if (!ipc_operation.BelongsTo(ipc_generation))
			{
				CloseIpcPipe();
				return IpcPollResult::Error;
			}

			if ((ipc_transport_state == IpcTransportState::Reading ||
				ipc_transport_state == IpcTransportState::Writing ||
				ipc_transport_state == IpcTransportState::AwaitingClientAck) &&
				GetTickCount64() >= ipc_io_deadline)
			{
				return BeginIpcClose();
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
				if (!ipc_operation.Complete(ipc_generation))
					return IpcPollResult::Error;
				if (ipc_transport_state == IpcTransportState::AwaitingClientAck &&
					(error == ERROR_BROKEN_PIPE || error == ERROR_NO_DATA))
					return ReconnectIpcPipe();

				CloseIpcPipe();
				return error == ERROR_BROKEN_PIPE || error == ERROR_NO_DATA
					? IpcPollResult::Idle
					: IpcPollResult::Error;
			}
			if (!ipc_operation.Complete(ipc_generation))
				return IpcPollResult::Error;

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
				return BeginIpcClientAck();
			case IpcTransportState::AwaitingClientAck:
				if (transferred != sizeof(ipc_delivery_ack) ||
					!IsExpectedDeliveryAck())
				{
					CloseIpcPipe();
					return IpcPollResult::Error;
				}
				return ReconnectIpcPipe();
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
			case IpcKernelService::QueryCharacterEffects:
				if (!IsEmptyQueryRequest(request))
					return NTE_MODS_STATUS_INVALID_IPC_REQUEST;
				response.record_count = CopyCharacterEffects(
					response.payload.character_effects,
					NTE_CHARACTER_EFFECT_MAX);
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

			if (!ResetIpcOverlapped() ||
				!ipc_operation.Begin(ipc_generation))
			{
				CloseIpcPipe();
				return IpcPumpResult::Error;
			}
			DWORD bytes_written = 0;
			if (WriteFile(
				ipc_pipe,
				&ipc_response,
				sizeof(ipc_response),
				&bytes_written,
				&ipc_overlapped))
			{
				if (!ipc_operation.Complete(ipc_generation) ||
					bytes_written != sizeof(ipc_response))
				{
					CloseIpcPipe();
					return IpcPumpResult::Error;
				}

				return BeginIpcClientAck() == IpcPollResult::Error
					? IpcPumpResult::Error
					: IpcPumpResult::Processed;
			}

			if (GetLastError() != ERROR_IO_PENDING)
			{
				if (!ipc_operation.Complete(ipc_generation))
					return IpcPumpResult::Error;
				CloseIpcPipe();
				return IpcPumpResult::Error;
			}

			ipc_transport_state = IpcTransportState::Writing;
			ipc_io_deadline = GetTickCount64() + IPC_CLIENT_IO_TIMEOUT_MS;
			return IpcPumpResult::Processed;
		}

	} // namespace

	bool OpenRuntimePresence()
	{
		RuntimePresenceGuard guard;
		if (runtime_presence_event != nullptr)
			return true;

		LocalIpcSecurityAttributes security;
		TokenIdentity server_identity{};
		if (!security.Initialize(
				IPC_PRESENCE_CLIENT_ACCESS,
				WinMediumLabelSid,
				&server_identity))
			return false;

		const auto event_name = NTE_OBFUSCATE_STRING(
			NTE_MODS_RUNTIME_PRESENCE_NAME);
		// For a newly created object, CreateEventExW grants the requested server
		// handle access while the DACL governs later opens. The published DACL can
		// therefore expose only SYNCHRONIZE. Any pre-existing fixed-name object is
		// rejected rather than trusted or repaired in place.
		SetLastError(ERROR_SUCCESS);
		const HANDLE event = CreateEventExW(
			security.Get(),
			event_name.c_str(),
			CREATE_EVENT_MANUAL_RESET | CREATE_EVENT_INITIAL_SET,
			IPC_PRESENCE_OWNER_ACCESS);
		const DWORD create_error = GetLastError();
		if (event == nullptr)
			return false;
		if (create_error == ERROR_ALREADY_EXISTS ||
			!ValidatePresenceEventSecurity(event, server_identity) ||
			!SetEvent(event))
		{
			CloseHandle(event);
			return false;
		}
		runtime_presence_event = event;
		return true;
	}

	RuntimePresenceCloseResult CloseRuntimePresence()
	{
		RuntimePresenceGuard guard;
		if (runtime_presence_event == nullptr)
			return RuntimePresenceCloseResult::Closed;

		if (!CloseHandle(runtime_presence_event))
			return RuntimePresenceCloseResult::CloseFailed;
		runtime_presence_event = nullptr;
		return RuntimePresenceCloseResult::Closed;
	}

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
		IpcTransportTryGuard guard;
		if (!guard.Acquired())
			return IpcPumpResult::Idle;
		if (ipc_stopping)
			return IpcPumpResult::Idle;
		// Revalidate after taking the transport lock. A tick that snapshotted the
		// old capability before a workspace reload must not reopen a pipe that the
		// worker just closed. Lock order remains transport -> program.
		if ((runtime::EnabledCapabilities() & runtime::CAPABILITY_IPC) == 0)
			return IpcPumpResult::Idle;

		const IpcPollResult poll_result = PollIpcRequest();
		if (poll_result == IpcPollResult::Error)
			return IpcPumpResult::Error;
		if (poll_result == IpcPollResult::Idle)
			return IpcPumpResult::Idle;
		return CompleteIpcRequest(context);
	}

	void SetIpcStopping(bool stopping)
	{
		IpcTransportGuard guard;
		ipc_stopping = stopping;
	}

	IpcCloseResult CloseIpc()
	{
		IpcTransportGuard guard;
		return CloseIpcPipe()
			? IpcCloseResult::Closed
			: IpcCloseResult::DrainFailed;
	}
} // namespace nte::mods
