#pragma once

namespace nte::hook
{
enum class ProtectedPointerPatchCode
{
    Applied,
    ExpectedValueChanged,
    TargetInvalid,
    MakeWritableFailed,
    RolledBack,
    ProtectionRestoreFailed,
    RollbackFailed,
};

enum class ProtectedPointerSlotState
{
    Expected,
    Replacement,
    Other,
    Unknown,
};

struct ProtectedPointerPatchResult
{
    ProtectedPointerPatchCode code{};
    ProtectedPointerSlotState slot_state{};
    bool protection_restored{};
    // True once this invocation successfully published replacement, even when
    // it subsequently rolled the slot back. A retained immutable binding is
    // then required for a dispatch that already captured the replacement.
    bool replacement_was_published{};

    constexpr bool Applied() const noexcept
    {
        return code == ProtectedPointerPatchCode::Applied &&
            slot_state == ProtectedPointerSlotState::Replacement &&
            protection_restored;
    }

    constexpr bool RequiresFailClosed() const noexcept
    {
        return !protection_restored ||
            code == ProtectedPointerPatchCode::ExpectedValueChanged ||
            code == ProtectedPointerPatchCode::RollbackFailed;
    }

    constexpr bool RequiresBindingRetention() const noexcept
    {
        return replacement_was_published ||
            slot_state == ProtectedPointerSlotState::Replacement;
    }
};

constexpr ProtectedPointerSlotState ClassifyProtectedPointerSlot(
    void* observed,
    void* expected,
    void* replacement) noexcept
{
    if (observed == expected)
        return ProtectedPointerSlotState::Expected;
    if (observed == replacement)
        return ProtectedPointerSlotState::Replacement;
    return ProtectedPointerSlotState::Other;
}

// The callbacks keep the policy independent from Win32 so protection failures
// and the rollback path can be tested deterministically. compare_exchange must
// provide InterlockedCompareExchangePointer semantics and return the observed
// value before the attempted exchange.
template <typename MakeWritable, typename RestoreProtection, typename CompareExchange>
constexpr ProtectedPointerPatchResult ReplaceProtectedPointer(
    void* expected,
    void* replacement,
    MakeWritable&& make_writable,
    RestoreProtection&& restore_protection,
    CompareExchange&& compare_exchange)
{
    if (!make_writable())
    {
        return {
            ProtectedPointerPatchCode::MakeWritableFailed,
            ProtectedPointerSlotState::Unknown,
            true,
            false,
        };
    }

    void* observed = compare_exchange(replacement, expected);
    if (observed != expected)
    {
        const ProtectedPointerSlotState slot_state =
            ClassifyProtectedPointerSlot(observed, expected, replacement);
        const bool restored = restore_protection() || restore_protection();
        return {
            restored
                ? ProtectedPointerPatchCode::ExpectedValueChanged
                : ProtectedPointerPatchCode::ProtectionRestoreFailed,
            slot_state,
            restored,
            false,
        };
    }

    if (restore_protection())
    {
        return {
            ProtectedPointerPatchCode::Applied,
            ProtectedPointerSlotState::Replacement,
            true,
            true,
        };
    }

    // The mutation is not committed until the original page protection is
    // restored. Roll the slot back while it is still writable, then make one
    // final protection-restoration attempt.
    observed = compare_exchange(expected, replacement);
    const ProtectedPointerSlotState slot_state = observed == replacement
        ? ProtectedPointerSlotState::Expected
        : ClassifyProtectedPointerSlot(observed, expected, replacement);
    const bool restored = restore_protection();
    if (!restored)
    {
        return {
            ProtectedPointerPatchCode::ProtectionRestoreFailed,
            slot_state,
            false,
            true,
        };
    }
    return {
        slot_state == ProtectedPointerSlotState::Expected
            ? ProtectedPointerPatchCode::RolledBack
            : ProtectedPointerPatchCode::RollbackFailed,
        slot_state,
        true,
        true,
    };
}
} // namespace nte::hook
