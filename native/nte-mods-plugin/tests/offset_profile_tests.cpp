#include "../src/offset_resolver.hpp"

#include <cstdint>
#include <cstdio>

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
    std::printf(
        "OFFSET_PROFILE_TEST image_size=0x1000C000 production_checksum=0x0F77E695 production_known=%s test_checksum=0x0F77CA48 test_known=%s small_update_checksum=0x12345678 small_update_known=%s\n",
        production_known ? "true" : "false",
        test_known ? "true" : "false",
        small_update_known ? "true" : "false");
    return 0;
}
