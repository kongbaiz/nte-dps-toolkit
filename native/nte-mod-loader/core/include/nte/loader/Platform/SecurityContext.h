#pragma once

namespace nte::loader {

enum class AdministratorStatus {
	Administrator,
	NotAdministrator,
	ProbeFailed,
};

class SecurityContext final {
public:
	static AdministratorStatus Administrator();
    static bool EnableDebugPrivilege();
};

} // namespace nte::loader
