#include "nte/loader/Platform/SecurityContext.h"

#include <Windows.h>

namespace nte::loader {

AdministratorStatus SecurityContext::Administrator() {
    SID_IDENTIFIER_AUTHORITY ntAuthority = SECURITY_NT_AUTHORITY;
    PSID administrators = nullptr;
    BOOL isMember = FALSE;
    if (!AllocateAndInitializeSid(&ntAuthority, 2, SECURITY_BUILTIN_DOMAIN_RID,
                                  DOMAIN_ALIAS_RID_ADMINS, 0, 0, 0, 0, 0, 0,
                                  &administrators)) {
		return AdministratorStatus::ProbeFailed;
    }
	const BOOL checked = CheckTokenMembership(nullptr, administrators, &isMember);
    FreeSid(administrators);
	if (!checked) return AdministratorStatus::ProbeFailed;
	return isMember == TRUE
		? AdministratorStatus::Administrator
		: AdministratorStatus::NotAdministrator;
}

bool SecurityContext::EnableDebugPrivilege() {
    HANDLE rawToken = nullptr;
    if (!OpenProcessToken(GetCurrentProcess(), TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY, &rawToken)) {
        return false;
    }
    LUID luid{};
    const BOOL found = LookupPrivilegeValueA(nullptr, "SeDebugPrivilege", &luid);
    if (!found) {
        CloseHandle(rawToken);
        return false;
    }
    TOKEN_PRIVILEGES privileges{};
    privileges.PrivilegeCount = 1;
    privileges.Privileges[0].Luid = luid;
    privileges.Privileges[0].Attributes = SE_PRIVILEGE_ENABLED;
    SetLastError(ERROR_SUCCESS);
    const BOOL adjusted = AdjustTokenPrivileges(rawToken, FALSE, &privileges, 0, nullptr, nullptr);
    const DWORD error = GetLastError();
    CloseHandle(rawToken);
    return adjusted && error == ERROR_SUCCESS;
}

} // namespace nte::loader
