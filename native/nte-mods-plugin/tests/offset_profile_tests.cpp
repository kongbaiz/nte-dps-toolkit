#include "../src/offset_resolver.hpp"

#include <array>
#include <cstring>
#include <cstdint>
#include <cstdio>

namespace
{
    template <typename T, size_t Size>
    void Store(std::array<uint8_t, Size>& buffer, size_t offset, T value)
    {
        std::memcpy(buffer.data() + offset, &value, sizeof(value));
    }

    bool DirectFNamePoolDecodeWorks()
    {
        alignas(8) std::array<uint8_t, 0x40> pool{};
        alignas(8) std::array<uint8_t, 0x40> block{};
        Store<uint32_t>(pool, 0x08, 0);
        Store<uint32_t>(pool, 0x0C, 0x20);
        Store<uintptr_t>(pool, 0x10, reinterpret_cast<uintptr_t>(block.data()));
        Store<uint16_t>(block, 0, static_cast<uint16_t>(4U << 6));
        std::memcpy(block.data() + 2, "None", 4);

        wchar_t decoded[16]{};
        size_t decoded_length = 0;
        if (!nte::mods::offsets::detail::DecodeNameFromPool(
                reinterpret_cast<uintptr_t>(pool.data()),
                0,
                2,
                decoded,
                std::size(decoded),
                decoded_length))
            return false;
        return decoded_length == 6 &&
            decoded[0] == L'N' && decoded[1] == L'o' &&
            decoded[2] == L'n' && decoded[3] == L'e' &&
            decoded[4] == L'_' && decoded[5] == L'1' &&
            !nte::mods::offsets::detail::DecodeNameFromPool(
                reinterpret_cast<uintptr_t>(pool.data()),
                0x00010000,
                0,
                decoded,
                std::size(decoded),
            decoded_length);
    }

    bool AmbiguousGWorldUsesUniqueCodeReferenceWinner()
    {
        std::array<uint8_t, 0x40> code{};
        constexpr uintptr_t code_address = 0x140001000;
        constexpr uintptr_t preferred = 0x140300100;
        constexpr uintptr_t alias = 0x140300180;
        const auto write_reference = [&](size_t offset, uintptr_t target)
        {
            code[offset] = 0x48;
            code[offset + 1] = 0x8B;
            code[offset + 2] = 0x05;
            const int64_t displacement = static_cast<int64_t>(target) -
                static_cast<int64_t>(code_address + offset + 7);
            const int32_t displacement32 = static_cast<int32_t>(displacement);
            std::memcpy(code.data() + offset + 3, &displacement32, sizeof(displacement32));
        };
        write_reference(0x00, preferred);
        write_reference(0x10, preferred);
        write_reference(0x20, alias);

        const nte::mods::offsets::detail::SectionView section{
            code.data(), code.size(), code_address, true, false };
        const uintptr_t candidates[]{ preferred, alias };
        uintptr_t selected = 0;
        size_t reference_count = 0;
        if (!nte::mods::offsets::detail::SelectUniqueGWorldCandidateByCodeReferences(
                &section,
                1,
                candidates,
                std::size(candidates),
                selected,
                reference_count) ||
            selected != preferred || reference_count != 2)
            return false;

        std::array<uint8_t, 0x20> tied_code{};
        const auto write_tied_reference = [&](size_t offset, uintptr_t target)
        {
            tied_code[offset] = 0x48;
            tied_code[offset + 1] = 0x8B;
            tied_code[offset + 2] = 0x05;
            const int64_t displacement = static_cast<int64_t>(target) -
                static_cast<int64_t>(code_address + offset + 7);
            const int32_t displacement32 = static_cast<int32_t>(displacement);
            std::memcpy(
                tied_code.data() + offset + 3,
                &displacement32,
                sizeof(displacement32));
        };
        write_tied_reference(0x00, preferred);
        write_tied_reference(0x10, alias);
        const nte::mods::offsets::detail::SectionView tied_section{
            tied_code.data(), tied_code.size(), code_address, true, false };
        return !nte::mods::offsets::detail::SelectUniqueGWorldCandidateByCodeReferences(
            &tied_section,
            1,
            candidates,
            std::size(candidates),
            selected,
            reference_count);
    }
}

int main()
{
    constexpr size_t image_size = 0x1000C000;
    constexpr uint32_t production_checksum = 0x0F77E695;
    constexpr uint32_t test_checksum = 0x0F77CA48;
    constexpr uint32_t small_update_checksum = 0x12345678;
    const bool production_known =
        nte::mods::offsets::IsKnownImageProfile(
            image_size, production_checksum);
    const bool test_known =
        nte::mods::offsets::IsKnownImageProfile(
            image_size, test_checksum);
    const bool small_update_known =
        nte::mods::offsets::IsKnownImageProfile(
            image_size, small_update_checksum);
    const bool current_cn_known =
        nte::mods::offsets::IsKnownImageProfile(
            0x1066A000, 0x0FDCF5DD);
    std::printf(
        "OFFSET_PROFILE_TEST image_size=0x1000C000 production_checksum=0x0F77E695 production_known=%s test_checksum=0x0F77CA48 test_known=%s small_update_checksum=0x12345678 small_update_known=%s\n",
        production_known ? "true" : "false",
        test_known ? "true" : "false",
        small_update_known ? "true" : "false");
    std::printf(
        "FAST_PROFILE_TEST image_size=0x1066A000 checksum=0x0FDCF5DD current_cn_known=%s expected_viewport_tick=100\n",
        current_cn_known ? "true" : "false");
    const bool direct_fname_decode = DirectFNamePoolDecodeWorks();
    std::printf(
        "FAST_SEMANTIC_TEST direct_fname_decode=%s malformed_index_rejected=%s\n",
        direct_fname_decode ? "true" : "false",
        direct_fname_decode ? "true" : "false");
    const bool unique_code_reference_winner =
        AmbiguousGWorldUsesUniqueCodeReferenceWinner();
    std::printf(
        "GWORLD_AMBIGUITY_TEST unique_code_reference_winner=%s tied_score_rejected=%s\n",
        unique_code_reference_winner ? "true" : "false",
        unique_code_reference_winner ? "true" : "false");
    return direct_fname_decode && unique_code_reference_winner && current_cn_known ? 0 : 1;
}
