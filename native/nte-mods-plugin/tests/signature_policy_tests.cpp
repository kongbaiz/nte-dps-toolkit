#include "../src/signature_policy.hpp"
#include "../src/ipc_transport_policy.hpp"
#include "../src/offset_signatures.hpp"
#include "../src/viewport_hook_policy.hpp"

#include <array>
#include <cstddef>
#include <cstdint>

namespace
{
	constexpr int VIEWPORT_A = 1;
	constexpr int VIEWPORT_B = 2;
	constexpr int ORIGINAL_A = 3;
	constexpr int ORIGINAL_B = 4;

	constexpr bool OverlappedEpochCannotBeReusedBeforeCompletion()
	{
		nte::mods::ipc::OperationEpoch operation;
		return operation.Begin(7) && !operation.Begin(7) &&
			!operation.CanReuse() && operation.BelongsTo(7);
	}

	constexpr bool OverlappedEpochRejectsStaleCompletion()
	{
		nte::mods::ipc::OperationEpoch operation;
		return operation.Begin(7) && !operation.Complete(6) &&
			operation.BelongsTo(7) && operation.Complete(7) &&
			operation.CanReuse() && operation.Begin(8);
	}

	constexpr bool ViewportOriginalIsBoundPerObject()
	{
		constexpr std::array<nte::hook::ViewportOriginalBinding, 2> bindings{{
			{&VIEWPORT_A, &ORIGINAL_A},
			{&VIEWPORT_B, &ORIGINAL_B},
		}};
		return nte::hook::FindViewportOriginal(
			bindings.data(), bindings.size(), &VIEWPORT_A) == &ORIGINAL_A &&
			nte::hook::FindViewportOriginal(
				bindings.data(), bindings.size(), &VIEWPORT_B) == &ORIGINAL_B &&
			nte::hook::CanReuseViewportBinding(
				bindings[0], &VIEWPORT_A, &ORIGINAL_A) &&
			!nte::hook::CanReuseViewportBinding(
				bindings[0], &VIEWPORT_A, &ORIGINAL_B);
	}

	constexpr std::array<uint8_t, 6> PATTERN_BYTES{
		0x48, 0x81, 0xEC, 0x00, 0x00, 0x00,
	};
	constexpr std::array<uint8_t, 6> PATTERN_MASK{
		0xFF, 0xFF, 0xFF, 0x00, 0x00, 0x00,
	};
	constexpr nte::signature::BytePattern PATTERN{
		PATTERN_BYTES.data(),
		PATTERN_MASK.data(),
		PATTERN_BYTES.size(),
	};

	constexpr bool MaskedBytesIgnoreVersionSpecificOperands()
	{
		constexpr std::array<uint8_t, 8> input{
			0x90, 0x48, 0x81, 0xEC, 0x30, 0x01, 0x00, 0x90,
		};
		return nte::signature::Find(
			input.data(), input.size(), 0, input.size(), PATTERN) == 1;
	}

	constexpr bool FixedOpcodeChangesAreRejected()
	{
		constexpr std::array<uint8_t, 6> input{
			0x48, 0x83, 0xEC, 0x30, 0x01, 0x00,
		};
		return !nte::signature::Matches(
			input.data(), input.size(), PATTERN);
	}

	constexpr bool UniqueCandidateIsSelected()
	{
		constexpr std::array<bool, 5> candidates{
			false, false, true, false, false,
		};
		size_t selected = 0;
		return nte::signature::SelectUniqueIndex(
			0,
			candidates.size() - 1,
			[&](size_t index) { return candidates[index]; },
			selected) == nte::signature::SelectionResult::Unique &&
			selected == 2;
	}

	constexpr bool AmbiguousCandidatesFailClosed()
	{
		constexpr std::array<bool, 5> candidates{
			false, true, false, true, false,
		};
		size_t selected = 0;
		return nte::signature::SelectUniqueIndex(
			0,
			candidates.size() - 1,
			[&](size_t index) { return candidates[index]; },
			selected) == nte::signature::SelectionResult::Ambiguous;
	}

	constexpr bool PreferredSemanticViewportTickWinsGenericAmbiguity()
	{
		constexpr std::array<bool, 17> candidates{
			false, false, false, false, false, false, false, false, true,
			false, false, true, false, false, false, true, false,
		};
		size_t selected = 0;
		return nte::hook::SelectPreferredSemanticViewportTick(
			8,
			0,
			candidates.size() - 1,
			8,
			[&](size_t index) { return candidates[index]; },
			selected) && selected == 8;
	}

	constexpr bool NearestShiftedViewportTickIsSelected()
	{
		constexpr std::array<bool, 17> candidates{
			false, false, false, false, true, false, false, false, false,
			false, false, true, false, false, false, true, false,
		};
		size_t selected = 0;
		return nte::hook::SelectPreferredSemanticViewportTick(
			8,
			0,
			candidates.size() - 1,
			8,
			[&](size_t index) { return candidates[index]; },
			selected) && selected == 11;
	}

	constexpr bool EquidistantShiftedViewportTicksFailClosed()
	{
		constexpr std::array<bool, 17> candidates{
			false, false, false, false, false, true, false, false, false,
			false, false, true, false, false, false, false, false,
		};
		size_t selected = 0;
		return !nte::hook::SelectPreferredSemanticViewportTick(
			8,
			0,
			candidates.size() - 1,
			8,
			[&](size_t index) { return candidates[index]; },
			selected);
	}

	constexpr std::array<uint8_t, 111> APPEND_NAME_READBACK{
		0x48, 0x89, 0x5C, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x57, 0x48,
		0x83, 0xEC, 0x20, 0x80, 0x3D, 0x82, 0x51, 0xD6, 0x0D, 0x00, 0x48, 0x8B,
		0xFA, 0x8B, 0x19, 0x48, 0x8B, 0xF1, 0x74, 0x09, 0x48, 0x8D, 0x15, 0x19,
		0x54, 0xD6, 0x0D, 0xEB, 0x16, 0x48, 0x8D, 0x0D, 0x10, 0x54, 0xD6, 0x0D,
		0xE8, 0xDB, 0xE3, 0xFE, 0xFF, 0x48, 0x8B, 0xD0, 0xC6, 0x05, 0x59, 0x51,
		0xD6, 0x0D, 0x01, 0x8B, 0xCB, 0x0F, 0xB7, 0xC3, 0xC1, 0xE9, 0x10, 0x89,
		0x4C, 0x24, 0x30, 0x89, 0x44, 0x24, 0x34, 0x48, 0x8B, 0x44, 0x24, 0x30,
		0x48, 0xC1, 0xE8, 0x20, 0x8D, 0x1C, 0x00, 0x48, 0x03, 0x5C, 0xCA, 0x10,
		0x48, 0x8B, 0xCF, 0x0F, 0xB7, 0x13, 0xC1, 0xEA, 0x06, 0x83, 0x7E, 0x04,
		0x00, 0x75, 0x1F,
	};

	constexpr std::array<uint8_t, 35> GWORLD_READBACK{
		0x48, 0x8B, 0x04, 0xD0, 0x8B, 0x04, 0x01, 0x39, 0x05, 0x16, 0xE1, 0xD0,
		0x0B, 0x7F, 0x14, 0x48, 0x89, 0x1D, 0xAD, 0x94, 0x79, 0x0B, 0x48, 0x8D,
		0x05, 0x76, 0x94, 0x79, 0x0B, 0x48, 0x83, 0xC4, 0x20, 0x5B, 0xC3,
	};

	constexpr bool LiveAppendNameSignatureMatchesReadback()
	{
		return nte::signature::Matches(
			APPEND_NAME_READBACK.data(),
			APPEND_NAME_READBACK.size(),
			nte::mods::offsets::detail::APPEND_NAME_LIVE_SIGNATURE);
	}

	constexpr bool LiveAppendNameSignatureMasksRelocations()
	{
		auto input = APPEND_NAME_READBACK;
		constexpr std::array<size_t, 23> wildcard_indices{
			17, 18, 19, 20, 31, 35, 36, 37, 38, 40, 44, 45, 46,
			47, 49, 50, 51, 52, 58, 59, 60, 61, 110,
		};
		for (size_t index = 0; index < wildcard_indices.size(); ++index)
			input[wildcard_indices[index]] = static_cast<uint8_t>(index + 1);
		return nte::signature::Matches(
			input.data(),
			input.size(),
			nte::mods::offsets::detail::APPEND_NAME_LIVE_SIGNATURE);
	}

	constexpr bool LiveGWorldSignatureMatchesReadback()
	{
		return nte::signature::Matches(
			GWORLD_READBACK.data(),
			GWORLD_READBACK.size(),
			nte::mods::offsets::detail::GWORLD_SEQUENCE);
	}

	constexpr bool LiveGWorldSignatureMasksRelocations()
	{
		auto input = GWORLD_READBACK;
		for (size_t index = 9; index <= 12; ++index)
			input[index] = static_cast<uint8_t>(index + 1);
		for (size_t index = 18; index <= 21; ++index)
			input[index] = static_cast<uint8_t>(index + 1);
		for (size_t index = 25; index <= 28; ++index)
			input[index] = static_cast<uint8_t>(index + 1);
		return nte::signature::Matches(
			input.data(),
			input.size(),
			nte::mods::offsets::detail::GWORLD_SEQUENCE);
	}

	static_assert(MaskedBytesIgnoreVersionSpecificOperands());
	static_assert(OverlappedEpochCannotBeReusedBeforeCompletion());
	static_assert(OverlappedEpochRejectsStaleCompletion());
	static_assert(ViewportOriginalIsBoundPerObject());
	static_assert(FixedOpcodeChangesAreRejected());
	static_assert(UniqueCandidateIsSelected());
	static_assert(AmbiguousCandidatesFailClosed());
	static_assert(PreferredSemanticViewportTickWinsGenericAmbiguity());
	static_assert(NearestShiftedViewportTickIsSelected());
	static_assert(EquidistantShiftedViewportTicksFailClosed());
	static_assert(LiveAppendNameSignatureMatchesReadback());
	static_assert(LiveAppendNameSignatureMasksRelocations());
	static_assert(LiveGWorldSignatureMatchesReadback());
	static_assert(LiveGWorldSignatureMasksRelocations());
} // namespace
