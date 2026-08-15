#include "find_offsets_scanner.hpp"

#include <Windows.h>

#include <algorithm>
#include <array>
#include <atomic>
#include <cctype>
#include <cstdint>
#include <cstdio>
#include <cwchar>
#include <cstring>
#include <exception>
#include <mutex>
#include <optional>
#include <sstream>
#include <stdexcept>
#include <string>
#include <thread>
#include <tuple>
#include <unordered_map>
#include <utility>
#include <vector>

namespace nte::mods::offsets::find_offsets
{
namespace
{

constexpr size_t MAX_MEMORY_REGIONS = 262144;
class NullLogStream
{
public:
    template <typename T>
    NullLogStream& operator<<(const T&) noexcept { return *this; }
};

NullLogStream NullLog() noexcept { return {}; }

static uint16_t u16(const uint8_t* d, size_t off) { uint16_t v; std::memcpy(&v, d + off, 2); return v; }
static uint32_t u32(const uint8_t* d, size_t off) { uint32_t v; std::memcpy(&v, d + off, 4); return v; }
static int32_t i32(const uint8_t* d, size_t off) { int32_t v; std::memcpy(&v, d + off, 4); return v; }
static uint64_t u64(const uint8_t* d, size_t off) { uint64_t v; std::memcpy(&v, d + off, 8); return v; }
static bool is_ptr(uint64_t value)
{
    return value >= 0x10000ULL && value < 0x0000800000000000ULL && (value % 8) == 0;
}
static std::optional<uint64_t> TryRipRelativeTargetImpl(
    const uint8_t* code, size_t size, uint64_t instructionAddress)
{
    if (!code || size < 6) return std::nullopt;
    size_t opcodeOffset = 0;
    if ((code[0] & 0xF0) == 0x40) opcodeOffset = 1;
    if (size < opcodeOffset + 6) return std::nullopt;
    const uint8_t opcode = code[opcodeOffset];
    if (opcode != 0x8B && opcode != 0x89 && opcode != 0x8D &&
        opcode != 0x3B && opcode != 0x39) return std::nullopt;
    if ((code[opcodeOffset + 1] & 0xC7) != 0x05) return std::nullopt;
    const int32_t displacement = i32(code, opcodeOffset + 2);
    const uint64_t instructionEnd = instructionAddress + opcodeOffset + 6;
    if (displacement >= 0) {
        if ((uint64_t)displacement > UINT64_MAX - instructionEnd) return std::nullopt;
        return instructionEnd + (uint64_t)displacement;
    }
    const uint64_t magnitude = (uint64_t)(-(int64_t)displacement);
    if (magnitude > instructionEnd) return std::nullopt;
    return instructionEnd - magnitude;
}
static std::string HexU64(uint64_t v)
{
    char buf[24];
    std::snprintf(buf, sizeof(buf), "0x%llx", static_cast<unsigned long long>(v));
    return buf;
}
static std::string ToLower(std::string s)
{
    std::transform(s.begin(), s.end(), s.begin(), [](unsigned char value) {
        return static_cast<char>(std::tolower(value));
    });
    return s;
}

static std::string WideToUtf8(const std::wstring& text)
{
    if (text.empty()) return {};
    const int required = WideCharToMultiByte(
        CP_UTF8,
        0,
        text.data(),
        static_cast<int>(text.size()),
        nullptr,
        0,
        nullptr,
        nullptr);
    if (required <= 0) return {};
    std::string result(static_cast<size_t>(required), '\0');
    if (WideCharToMultiByte(
            CP_UTF8,
            0,
            text.data(),
            static_cast<int>(text.size()),
            result.data(),
            required,
            nullptr,
            nullptr) != required)
        return {};
    return result;
}
struct Region {
    uint64_t base = 0, end = 0;
    std::string protect, state, type;
    uint64_t allocationBase = 0;
    uint64_t size() const { return end - base; }
};

static std::string ProtectToString(DWORD p) {
    std::string s;
    DWORD base = p & 0xFF;
    switch (base) {
    case PAGE_NOACCESS:          s = "PAGE_NOACCESS"; break;
    case PAGE_READONLY:          s = "PAGE_READONLY"; break;
    case PAGE_READWRITE:         s = "PAGE_READWRITE"; break;
    case PAGE_WRITECOPY:         s = "PAGE_WRITECOPY"; break;
    case PAGE_EXECUTE:           s = "PAGE_EXECUTE"; break;
    case PAGE_EXECUTE_READ:      s = "PAGE_EXECUTE_READ"; break;
    case PAGE_EXECUTE_READWRITE: s = "PAGE_EXECUTE_READWRITE"; break;
    case PAGE_EXECUTE_WRITECOPY: s = "PAGE_EXECUTE_WRITECOPY"; break;
    default:                     s = "PAGE_UNKNOWN"; break;
    }
    if (p & PAGE_GUARD) s += "|PAGE_GUARD";
    if (p & PAGE_NOCACHE) s += "|PAGE_NOCACHE";
    return s;
}
static std::string StateToString(DWORD st) {
    if (st == MEM_COMMIT) return "MEM_COMMIT";
    if (st == MEM_FREE) return "MEM_FREE";
    if (st == MEM_RESERVE) return "MEM_RESERVE";
    return "MEM_UNKNOWN";
}
static std::string TypeToString(DWORD t) {
    if (t == MEM_IMAGE) return "MEM_IMAGE";
    if (t == MEM_MAPPED) return "MEM_MAPPED";
    if (t == MEM_PRIVATE) return "MEM_PRIVATE";
    return "";
}

// ---------------------------------------------------------------------------
// MemoryIO
//
// Live mode keeps two handles:
//   - hProcessQuery: a normal OpenProcess handle used for VirtualQueryEx/region
//     enumeration.
//   - hProcessRead: the handle used for ReadProcessMemory. It starts as the
//     normal handle, but callers may provide a distinct read handle.
//     DriverClient::OpenProcessRoot when a protected process denies VM reads.
//
// The direct driver protocol is no longer implemented here. All driver-assisted
// This embedded build reads the current process directly.
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// ParallelForEachRegion - speed optimization: the biggest cost centers in
// this tool (GObjects candidate scanning, GWorld pointer/reference search)
// are independent per-region ReadProcessMemory + scan operations. Windows
// handles concurrent ReadProcessMemory calls against the same handle from
// multiple threads without issue (each call targets a disjoint address
// range), so distributing regions across a small worker pool gives a
// close-to-linear speedup on multi-core machines with no correctness cost -
// each region is scanned independently and callers merge results under a
// mutex. Caps at 8 workers: ReadProcessMemory is the bottleneck (a kernel
// call, effectively I/O-bound), not local CPU, so more threads than that
// mostly just adds contention without helping throughput.
// ---------------------------------------------------------------------------
template <typename Fn>
static void ParallelFor(size_t n, Fn&& fn) {
    if (n == 0) return;
    unsigned hw = std::thread::hardware_concurrency();
    size_t workerCount = std::min<size_t>(std::max<unsigned>(hw, 1u), 8);
    workerCount = std::min(workerCount, n);
    if (workerCount <= 1) {
        for (size_t i = 0; i < n; i++) fn(i);
        return;
    }
    std::atomic<size_t> next{ 0 };
    std::vector<std::thread> workers;
    workers.reserve(workerCount);
    for (size_t w = 0; w < workerCount; w++) {
        workers.emplace_back([&]() {
            for (;;) {
                size_t i = next.fetch_add(1);
                if (i >= n) break;
                fn(i);
            }
            });
    }
    for (auto& t : workers) t.join();
}

template <typename Fn>
static void ParallelForEachRegion(const std::vector<Region>& regions, Fn&& fn) {
    ParallelFor(regions.size(), [&](size_t i) { fn(regions[i]); });
}

// ---------------------------------------------------------------------------
// FindAllOccurrences - SIMD-friendly substring scan. Instead of calling
// memcmp at EVERY offset (the previous approach: O(n*m) with a function call
// per byte, ~billions of calls across multi-GB of committed memory), use
// memchr to jump straight to first-byte candidates. memchr is vectorized in
// the CRT (scans 16-32 bytes per step), so for the sparse byte patterns this
// tool searches for this is typically 10-30x faster with identical results.
// ---------------------------------------------------------------------------
static void FindAllOccurrences(const uint8_t* hay, size_t hayLen,
    const uint8_t* needle, size_t needleLen,
    uint64_t baseAddr, std::vector<uint64_t>& out) {
    if (needleLen == 0 || hayLen < needleLen) return;
    const uint8_t first = needle[0];
    const uint8_t* const scanEnd = hay + (hayLen - needleLen) + 1; // last possible start + 1
    const uint8_t* p = hay;
    while (p < scanEnd) {
        const uint8_t* q = static_cast<const uint8_t*>(memchr(p, first, (size_t)(scanEnd - p)));
        if (!q) break;
        if (needleLen == 1 || memcmp(q + 1, needle + 1, needleLen - 1) == 0)
            out.push_back(baseAddr + (uint64_t)(q - hay));
        p = q + 1;
    }
}

class MemoryIO {
public:
    // Live-process mode. Region/layout queries go through hProcessQuery.
    // Memory content is read through hProcessRead; if that still fails, the
    // optional SDK client is used as a final DriverClient::ReadMemory fallback.
    MemoryIO(
        DWORD pid,
        HANDLE hProcessQuery,
        HANDLE hProcessRead,
        HANDLE cancellationEvent)
        : pid_(pid), hProcessQuery_(hProcessQuery), hProcess_(hProcessRead),
        cancellationEvent_(cancellationEvent) {
    }

    // Offline/file-dump mode: `dumpData` is a raw byte-for-byte capture of
    // one contiguous memory region (e.g. produced by OpenArk's own memory
    // viewer "dump to file"), and `dumpBase` is the virtual address that
    // dumpData[0] corresponds to. No live process, no driver, no MCP - pure
    // static analysis. Only addresses inside [dumpBase, dumpBase+size) are
    // readable; anything else throws, exactly as a real region boundary
    // would. This is necessarily a PARTIAL substitute for a live process:
    // if the dump only covers the main module's mapped image (as opposed to
    // heap memory), steps that need to dereference heap-allocated content
    // (e.g. FindGObjects validating individual UObject entries, FindGWorld)
    // will very likely fail with "outside dump range" - that's an expected,
    // honest limitation of static analysis on a partial dump, not a bug.
    MemoryIO(std::vector<uint8_t> dumpData, uint64_t dumpBase)
        : fileMode_(true), dumpBase_(dumpBase), dumpData_(std::move(dumpData)) {
    }

    // Core read into a caller-provided buffer (resized to the exact number of
    // bytes actually read). Reusing a single buffer across the many chunk reads
    // of a tight scan loop eliminates per-chunk heap allocation churn. Same
    // failure contract as Read(): throws on hard failure; on return buf.size()
    // is the byte count.
    size_t ReadInto(uint64_t address, size_t size, std::vector<uint8_t>& buf) {
        if (Cancelled()) throw std::runtime_error("find_offsets cancelled");
        if (fileMode_) {
            if (address < dumpBase_ || address - dumpBase_ > dumpData_.size() ||
                size > dumpData_.size() - (address - dumpBase_)) {
                throw std::runtime_error("address " + HexU64(address) + " (+" + std::to_string(size) +
                    " bytes) is outside the loaded dump range [" + HexU64(dumpBase_) +
                    ", " + HexU64(dumpBase_ + dumpData_.size()) + ")");
            }
            size_t off = (size_t)(address - dumpBase_);
            buf.assign(dumpData_.begin() + off, dumpData_.begin() + off + size);
            return size;
        }
        buf.resize(size);
        SIZE_T bytesRead = 0;
        if (ReadProcessMemory(hProcess_, (LPCVOID)address, buf.data(), size, &bytesRead)) {
            buf.resize(bytesRead);
            return bytesRead;
        }

        DWORD err = GetLastError();
        // ERROR_PARTIAL_COPY means the requested range only PARTIALLY lies in
        // valid/committed memory (common when a scan window straddles a
        // region boundary). Query the actual region the address falls in
        // (via the query-capable handle, since that's the one VirtualQueryEx
        // reliably works with) and retry clamped to what's actually mapped,
        // instead of failing the whole read outright.
        if (err == ERROR_PARTIAL_COPY) {
            MEMORY_BASIC_INFORMATION mbi{};
            if (VirtualQueryEx(hProcessQuery_, (LPCVOID)address, &mbi, sizeof(mbi)) && mbi.State == MEM_COMMIT) {
                uint64_t regionEnd = (uint64_t)mbi.BaseAddress + mbi.RegionSize;
                if (regionEnd > address) {
                    size_t clamped = (size_t)std::min<uint64_t>(size, regionEnd - address);
                    if (clamped > 0 && clamped < size) {
                        buf.resize(clamped);
                        if (ReadProcessMemory(hProcess_, (LPCVOID)address, buf.data(), clamped, &bytesRead)) {
                            buf.resize(bytesRead);
                            return bytesRead;
                        }
                    }
                }
            }
        }
        throw std::runtime_error("ReadProcessMemory failed at " + HexU64(address) + ", err=" + std::to_string(err));
    }

    std::vector<uint8_t> Read(uint64_t address, size_t size) {
        std::vector<uint8_t> buf;
        ReadInto(address, size, buf);
        return buf;
    }

    std::vector<Region> Regions() {
        if (fileMode_) {
            Region r;
            r.base = dumpBase_;
            r.end = dumpBase_ + dumpData_.size();
            r.allocationBase = dumpBase_;
            r.state = "MEM_COMMIT";
            r.type = "MEM_IMAGE";
            // Permissive on purpose: we don't know the real per-page
            // protections from a flat dump, and OffsetFinder's scanning
            // logic filters candidate regions by protect substrings
            // (READWRITE/EXECUTE/etc) - claiming all of them avoids
            // spuriously excluding real data purely because we can't tell
            // .text from .data in a raw dump.
            r.protect = "PAGE_EXECUTE_READWRITE";
            return { r };
        }
        std::vector<Region> regions;
        uint64_t addr = 0;
        MEMORY_BASIC_INFORMATION mbi;
        while (addr < 0x00007FFFFFFFFFFFULL) {
            SIZE_T n = VirtualQueryEx(hProcessQuery_, (LPCVOID)addr, &mbi, sizeof(mbi));
            if (n == 0) {
                if (addr == 0) NullLog() << "[.] VirtualQueryEx(addr=0) failed immediately, err=" << GetLastError() << "\n";
                break;
            }
            Region r;
            r.base = (uint64_t)mbi.BaseAddress;
            r.end = r.base + mbi.RegionSize;
            r.allocationBase = (uint64_t)mbi.AllocationBase;
            r.state = StateToString(mbi.State);
            r.type = TypeToString(mbi.Type);
            r.protect = (mbi.State == MEM_COMMIT) ? ProtectToString(mbi.Protect) : "";
            regions.push_back(r);
            if (regions.size() > MAX_MEMORY_REGIONS)
                throw std::runtime_error("memory region count exceeds find_offsets budget");
            if (mbi.RegionSize == 0) break;
            addr = r.end;
        }
        std::sort(regions.begin(), regions.end(), [](const Region& a, const Region& b) { return a.base < b.base; });
        return regions;
    }

    // Local re-implementation of the MCP "process_memory_search" tool:
    // scans committed, non-guard regions for a byte pattern.
    std::vector<uint64_t> Search(const std::vector<Region>& regions, const std::string& pattern,
        const std::string& patternType, size_t maxResults) {
        if (Cancelled()) return {};
        std::vector<uint8_t> needle;
        if (patternType == "hex") {
            std::string hex;
            for (char c : pattern) if (isxdigit((unsigned char)c)) hex += c;
            for (size_t i = 0; i + 1 < hex.size(); i += 2) {
                needle.push_back((uint8_t)strtol(hex.substr(i, 2).c_str(), nullptr, 16));
            }
        }
        else { // utf8
            needle.assign(pattern.begin(), pattern.end());
        }
        std::vector<uint64_t> hits;
        if (needle.empty()) return hits;

        std::vector<Region> scanCandidates;
        for (auto& r : regions) {
            if (r.state != "MEM_COMMIT") continue;
            if (r.protect.find("NOACCESS") != std::string::npos) continue;
            if (r.protect.find("PAGE_GUARD") != std::string::npos) continue;
            scanCandidates.push_back(r);
        }

        std::mutex hitsMutex;
        std::atomic<bool> satisfied{ false };
        const size_t kChunk = 8 * 1024 * 1024; // 8 MiB scan window
        const uint64_t kPage = 0x1000;

        ParallelForEachRegion(scanCandidates, [&](const Region& r) {
            if (Cancelled() || satisfied.load(std::memory_order_relaxed)) return;
            std::vector<uint64_t> localHits;
            std::vector<uint8_t> data; // reused across chunks: no per-chunk allocation
            uint64_t regionSize = r.size();
            uint64_t pos = 0;
            int consecutiveFailures = 0;
            while (pos < regionSize) {
                if (Cancelled() || satisfied.load(std::memory_order_relaxed)) break;
                size_t want = (size_t)std::min<uint64_t>(kChunk, regionSize - pos);
                try {
                    ReadInto(r.base + pos, want, data);
                }
                catch (const std::exception&) {
                    // A single bad/inaccessible page inside an otherwise-huge
                    // region shouldn't kill the scan of the rest of it (this
                    // is common on protected processes where some pages
                    // within a nominally-MEM_COMMIT region are individually
                    // unreadable) - skip forward one page and keep going,
                    // but give up on this region if we hit a long unbroken
                    // run of failures (avoids spending forever page-stepping
                    // across a huge genuinely-unreadable span).
                    consecutiveFailures++;
                    if (consecutiveFailures > 64) break;
                    pos += kPage;
                    continue;
                }
                consecutiveFailures = 0;
                if (data.size() < needle.size()) { pos += kPage; continue; }
                FindAllOccurrences(data.data(), data.size(), needle.data(), needle.size(), r.base + pos, localHits);
                // overlap by needle.size()-1 so matches spanning a chunk boundary aren't missed
                uint64_t advance = data.size() > needle.size() ? data.size() - needle.size() + 1 : data.size();
                pos += advance;
            }
            if (!localHits.empty()) {
                std::lock_guard<std::mutex> lock(hitsMutex);
                hits.insert(hits.end(), localHits.begin(), localHits.end());
                if (hits.size() >= maxResults) satisfied.store(true, std::memory_order_relaxed);
            }
            });

        // ACCURACY/DETERMINISM: workers append hits in nondeterministic
        // completion order, so an arbitrary-order truncate to maxResults could
        // drop the true (lowest-address) candidate. Sort by address first so
        // that (a) truncation keeps the lowest-address hits every callsite here
        // actually wants (they all min_element / pick-first), and (b) results
        // are reproducible run-to-run.
        std::sort(hits.begin(), hits.end());
        if (hits.size() > maxResults) hits.resize(maxResults);
        return hits;
    }

    bool Cancelled() const noexcept {
        return cancellationEvent_ != nullptr &&
            WaitForSingleObject(cancellationEvent_, 0) == WAIT_OBJECT_0;
    }

private:
    DWORD pid_ = 0;
    HANDLE hProcessQuery_ = nullptr; // VirtualQueryEx
    HANDLE hProcess_ = nullptr;      // ReadProcessMemory
    HANDLE cancellationEvent_ = nullptr;
    bool fileMode_ = false;
    uint64_t dumpBase_ = 0;
    std::vector<uint8_t> dumpData_;
};

// ---------------------------------------------------------------------------
// FNameDecoder - equivalent of the Python FNameDecoder
// ---------------------------------------------------------------------------
class FNameDecoder {
public:
    FNameDecoder(MemoryIO& mem, std::vector<uint64_t> blocks) : mem_(mem), blocks_(std::move(blocks)) {}

    std::optional<std::string> Get(int32_t index, uint32_t number = 0) {
        uint64_t key = ((uint64_t)(uint32_t)index << 32) | number;
        auto it = cache_.find(key);
        if (it != cache_.end()) return it->second;
        if (index < 0) return std::nullopt;
        size_t blockIdx = (size_t)((uint32_t)index >> 16);
        size_t inBlock = (size_t)((uint32_t)index & 0xFFFF) * 2;
        if (blockIdx >= blocks_.size() || blocks_[blockIdx] == 0) return std::nullopt;
        std::vector<uint8_t> entry;
        try {
            entry = mem_.Read(blocks_[blockIdx] + inBlock, 256);
        }
        catch (...) {
            cache_[key] = std::nullopt;
            return std::nullopt;
        }
        if (entry.size() < 2) { cache_[key] = std::nullopt; return std::nullopt; }
        uint16_t header = u16(entry.data(), 0);
        bool isWide = header & 1;
        int length = (header >> 6) & 0x3FF;
        if (length <= 0 || length > 240) { cache_[key] = std::nullopt; return std::nullopt; }
        size_t byteLen = (size_t)length * (isWide ? 2 : 1);
        if (entry.size() < 2 + byteLen) { cache_[key] = std::nullopt; return std::nullopt; }
        std::string text;
        if (isWide) {
            std::wstring w((size_t)length, 0);
            memcpy(w.data(), entry.data() + 2, byteLen);
            text = WideToUtf8(w);
        }
        else {
            text.assign((const char*)entry.data() + 2, length);
        }
        if (!text.empty() && number) text += "_" + std::to_string(number - 1);
        cache_[key] = text;
        return text;
    }

private:
    MemoryIO& mem_;
    std::vector<uint64_t> blocks_;
    std::unordered_map<uint64_t, std::optional<std::string>> cache_;
};

// ---------------------------------------------------------------------------
// ObjectInfo - equivalent of the dict returned by Python's object_info
// ---------------------------------------------------------------------------
struct ObjectInfo {
    uint64_t vtable = 0, cls = 0, outer = 0;
    uint32_t flags = 0, number = 0;
    int32_t index = 0, nameIdx = 0;
    std::string name;
};

// ---------------------------------------------------------------------------
// GObjects info - equivalent of the dict used throughout find_gobjects etc.
// ---------------------------------------------------------------------------
struct GObjectsInfo {
    uint64_t objects = 0;
    uint32_t maxElements = 0, numElements = 0, maxChunks = 0, numChunks = 0;
};

static constexpr uint32_t ELEMENTS_PER_CHUNK = 0x10000;
static constexpr uint32_t FUOBJECT_ITEM_SIZE = 0x18;
static constexpr uint32_t UOBJECT_SIZE = 0x28;

// ---------------------------------------------------------------------------
// OffsetFinder - direct port of the Python OffsetFinder class
// ---------------------------------------------------------------------------
class OffsetFinder {
public:
    OffsetFinder(MemoryIO& mem, DWORD pid, std::string processName, std::optional<uint64_t> imageBase)
        : mem_(mem), pid_(pid), processName_(ToLower(std::move(processName))) {
        regions_ = mem_.Regions();
        for (auto& r : regions_) regionBases_.push_back(r.base);
        NullLog() << "[.] OffsetFinder ctor: " << regions_.size() << " region(s)";
        if (!regions_.empty()) NullLog() << ", region[0]=" << HexU64(regions_[0].base) << "-" << HexU64(regions_[0].end)
            << " state=" << regions_[0].state << " protect=" << regions_[0].protect;
        NullLog() << "\n";
        imageBase_ = imageBase ? *imageBase : DetectImageBase();
        for (auto& r : regions_) if (r.allocationBase == imageBase_) moduleRegions_.push_back(r);
    }

    uint64_t ImageBase() const { return imageBase_; }
    uint64_t Rva(uint64_t va) const { return va - imageBase_; }

    const Region* RegionFor(uint64_t address) const {
        auto it = std::upper_bound(regionBases_.begin(), regionBases_.end(), address);
        if (it == regionBases_.begin()) return nullptr;
        size_t idx = (size_t)(it - regionBases_.begin()) - 1;
        const Region& r = regions_[idx];
        if (r.base <= address && address < r.end) return &r;
        return nullptr;
    }

    // --- find_fname_pool ---
    uint64_t FindFNamePool(std::vector<std::string>& evidence) {
        std::vector<std::tuple<uint64_t, uint32_t, uint32_t>> poolCandidates;
        uint64_t writableBytesScanned = 0;
        for (const Region& region : moduleRegions_) {
            if (region.state != "MEM_COMMIT") continue;
            const bool writable = region.protect.find("READWRITE") != std::string::npos ||
                region.protect.find("WRITECOPY") != std::string::npos ||
                region.protect.find("EXECUTE_READWRITE") != std::string::npos;
            if (!writable || region.size() < 0x20 || region.size() > 0x2000000) continue;
            std::vector<uint8_t> data;
            try { data = mem_.Read(region.base, (size_t)region.size()); }
            catch (...) { continue; }
            writableBytesScanned += data.size();
            for (size_t off = 0; off + 0x20 <= data.size(); off += 8) {
                const uint32_t currentBlock = u32(data.data(), off + 0x08);
                const uint32_t currentCursor = u32(data.data(), off + 0x0C);
                const uint64_t block0 = u64(data.data(), off + 0x10);
                if (currentBlock >= 0x2000 || currentCursor == 0 ||
                    currentCursor >= 0x20000 || !is_ptr(block0)) continue;
                std::vector<uint8_t> sample;
                try { sample = mem_.Read(block0, 0x100); }
                catch (...) { continue; }
                if (sample.size() < 6) continue;
                const uint16_t noneHeader = u16(sample.data(), 0);
                if ((noneHeader & 1) != 0 || ((noneHeader >> 6) & 0x3FF) != 4 ||
                    memcmp(sample.data() + 2, "None", 4) != 0 ||
                    !Contains(sample, "ByteProperty")) continue;
                poolCandidates.emplace_back(
                    region.base + off, currentBlock, currentCursor);
            }
        }
        std::sort(poolCandidates.begin(), poolCandidates.end());
        poolCandidates.erase(
            std::unique(poolCandidates.begin(), poolCandidates.end()),
            poolCandidates.end());
        if (poolCandidates.size() != 1) {
            throw std::runtime_error("Validated writable-section FNamePool scan found " +
                std::to_string(poolCandidates.size()) + " candidates; expected exactly one");
        }
        const auto& best = poolCandidates.front();
        evidence.push_back("single-pass writable module scan bytes=" +
            std::to_string(writableBytesScanned));
        evidence.push_back("block0 validates None/ByteProperty without a whole-process string search");
        evidence.push_back("FNamePool header current_block=" + std::to_string(std::get<1>(best)) +
            " current_cursor=" + HexU64(std::get<2>(best)));
        return std::get<0>(best);
    }

    FNameDecoder LoadFNamePool(uint64_t pool) {
        auto header = mem_.Read(pool, 0x1010);
        uint32_t currentBlock = u32(header.data(), 0x08);
        size_t count = std::min<size_t>((size_t)currentBlock + 1, 0x2000);
        std::vector<uint64_t> blocks;
        for (size_t i = 0; i < count; i++) blocks.push_back(u64(header.data(), 0x10 + i * 8));
        return FNameDecoder(mem_, std::move(blocks));
    }

    // --- find_gobjects ---
    uint64_t FindGObjects(FNameDecoder& names, GObjectsInfo& info, std::vector<std::string>& evidence) {
        struct Candidate {
            int score; uint64_t va, objects; uint32_t maxElements, numElements, maxChunks, numChunks;
            std::vector<std::string> decoded;
        };
        std::vector<Candidate> candidates;

        std::vector<Region> scanRegions;
        for (auto& r : moduleRegions_) {
            if (r.state != "MEM_COMMIT") continue;
            bool writable = r.protect.find("READWRITE") != std::string::npos ||
                r.protect.find("WRITECOPY") != std::string::npos ||
                r.protect.find("EXECUTE_READWRITE") != std::string::npos;
            if (!writable) continue;
            if (r.size() > 0x1000000) continue;
            scanRegions.push_back(r);
        }

        // FNameDecoder caches per (index,number) lookups internally but is
        // NOT synchronized - guard it with a mutex when called from multiple
        // worker threads below (cheap: contention is rare since most workers
        // decode disjoint name indices, and the guarded section is just a
        // couple of memory reads + hashmap lookup).
        std::mutex namesMutex;
        std::mutex candidatesMutex;
        static const std::vector<std::string> wanted = {
            "/Script/CoreUObject", "Object", "/Script/Engine", "Actor", "Class", "Package"
        };

        ParallelForEachRegion(scanRegions, [&](const Region& region) {
            std::vector<uint8_t> data;
            try { data = mem_.Read(region.base, (size_t)region.size()); }
            catch (...) { return; }
            if (data.size() < 0x20) return;
            std::vector<Candidate> localCandidates;
            for (size_t off = 0; off + 0x20 <= data.size(); off += 8) {
                uint64_t objects = u64(data.data(), off);
                if (!is_ptr(objects)) continue;
                uint32_t maxElements = u32(data.data(), off + 0x10);
                uint32_t numElements = u32(data.data(), off + 0x14);
                uint32_t maxChunks = u32(data.data(), off + 0x18);
                uint32_t numChunks = u32(data.data(), off + 0x1C);
                if (!(numElements >= 10000 && numElements <= maxElements && maxElements <= 30000000)) continue;
                if (!(numChunks >= 1 && numChunks <= maxChunks && maxChunks <= 8192)) continue;
                if ((uint64_t)numChunks * ELEMENTS_PER_CHUNK < numElements) continue;
                if (!RegionFor(objects)) continue;

                std::vector<uint8_t> ptrs, firstItems;
                try {
                    ptrs = mem_.Read(objects, (size_t)std::min<uint32_t>(numChunks, 16) * 8);
                    uint64_t firstChunk = u64(ptrs.data(), 0);
                    firstItems = mem_.Read(firstChunk, FUOBJECT_ITEM_SIZE * 32);
                }
                catch (...) { continue; }

                std::vector<std::string> decoded;
                int idxOk = 0;
                for (int i = 0; i < 32; i++) {
                    uint64_t obj = u64(firstItems.data(), i * FUOBJECT_ITEM_SIZE);
                    if (!is_ptr(obj)) continue;
                    std::vector<uint8_t> rawObj;
                    try { rawObj = mem_.Read(obj, UOBJECT_SIZE); }
                    catch (...) { continue; }
                    if (rawObj.size() < UOBJECT_SIZE) continue;
                    int32_t internalIndex = i32(rawObj.data(), 0x0C);
                    int32_t nameIndex = i32(rawObj.data(), 0x18);
                    uint32_t number = u32(rawObj.data(), 0x1C);
                    std::optional<std::string> name;
                    { std::lock_guard<std::mutex> lock(namesMutex); name = names.Get(nameIndex, number); }
                    if (internalIndex == i) idxOk++;
                    if (name) decoded.push_back(*name);
                }
                int score = idxOk * 5;
                bool hasObjectAnchor = false;
                for (auto& n : decoded) {
                    if (std::find(wanted.begin(), wanted.end(), n) != wanted.end()) score += 10;
                    if (n == "Object") hasObjectAnchor = true;
                }
                // ACCURACY: require the foundational "Object" name specifically
                // (not just any name from the wanted list) in addition to the
                // score threshold - "Object" is the root UClass of the entire
                // UE reflection hierarchy and is essentially always among the
                // first handful of GObjects entries in a real TUObjectArray;
                // requiring it specifically (rather than any 2 arbitrary
                // wanted-list hits) rejects candidates that scrape together
                // enough score from index-agreement noise plus a couple of
                // coincidental name matches without the one name that would
                // be present in every genuine UE build.
                if (score >= 20 && hasObjectAnchor) {
                    Candidate c;
                    c.score = score; c.va = region.base + off; c.objects = objects;
                    c.maxElements = maxElements; c.numElements = numElements;
                    c.maxChunks = maxChunks; c.numChunks = numChunks;
                    c.decoded.assign(decoded.begin(), decoded.begin() + std::min<size_t>(12, decoded.size()));
                    localCandidates.push_back(std::move(c));
                }
            }
            if (!localCandidates.empty()) {
                std::lock_guard<std::mutex> lock(candidatesMutex);
                for (auto& c : localCandidates) candidates.push_back(std::move(c));
            }
            });
        if (candidates.empty()) throw std::runtime_error("Could not find validated TUObjectArray");
        auto& best = *std::max_element(candidates.begin(), candidates.end(),
            [](const Candidate& a, const Candidate& b) { return a.score < b.score; });
        std::string decodedStr;
        for (size_t i = 0; i < std::min<size_t>(8, best.decoded.size()); i++) {
            if (i) decodedStr += ", ";
            decodedStr += best.decoded[i];
        }
        evidence.push_back("decoded first objects: " + decodedStr);
        info.objects = best.objects; info.maxElements = best.maxElements; info.numElements = best.numElements;
        info.maxChunks = best.maxChunks; info.numChunks = best.numChunks;
        return best.va;
    }

    std::optional<ObjectInfo> GetObjectInfo(uint64_t address, FNameDecoder& names) {
        std::vector<uint8_t> raw;
        try { raw = mem_.Read(address, UOBJECT_SIZE); }
        catch (...) { return std::nullopt; }
        if (raw.size() < UOBJECT_SIZE) return std::nullopt;
        uint64_t vtable = u64(raw.data(), 0);
        uint64_t cls = u64(raw.data(), 0x10);
        if (!is_ptr(vtable) || !is_ptr(cls)) return std::nullopt;
        int32_t nameIdx = i32(raw.data(), 0x18);
        uint32_t number = u32(raw.data(), 0x1C);
        auto name = names.Get(nameIdx, number);
        if (!name) return std::nullopt;
        ObjectInfo info;
        info.vtable = vtable; info.cls = cls;
        info.flags = u32(raw.data(), 8); info.index = i32(raw.data(), 0x0C);
        info.nameIdx = nameIdx; info.number = number; info.name = *name;
        info.outer = u64(raw.data(), 0x20);
        return info;
    }

    // --- find_process_event ---
    uint64_t FindProcessEvent(const GObjectsInfo& gobjInfo, int index, std::vector<std::string>& evidence) {
        // Only chunk0 is sampled here, so read just the first chunk pointer.
        auto chunk0Ptr = mem_.Read(gobjInfo.objects, 8);
        uint64_t firstChunk = u64(chunk0Ptr.data(), 0);
        // ACCURACY: sample more objects than the original 128 (up to 1024, or
        // fewer if chunk0 doesn't have that many) - a bigger sample makes the
        // majority-vtable-slot signal much more resistant to the rare object
        // whose vtable slot happens to coincidentally look executable (e.g. a
        // different virtual function that happens to also land in the
        // module's .text section).
        uint32_t sampleCount = std::min<uint32_t>(1024, gobjInfo.numElements);
        auto first = mem_.Read(firstChunk, (size_t)FUOBJECT_ITEM_SIZE * sampleCount);
        std::unordered_map<uint64_t, int> counts;
        int validSamples = 0;
        for (uint32_t i = 0; i < sampleCount; i++) {
            uint64_t obj = u64(first.data(), i * FUOBJECT_ITEM_SIZE);
            if (!is_ptr(obj)) continue;
            try {
                auto rawObj = mem_.Read(obj, 8);
                uint64_t vtable = u64(rawObj.data(), 0);
                if (!is_ptr(vtable)) continue;
                auto slot = mem_.Read(vtable + (uint64_t)index * 8, 8);
                uint64_t fn = u64(slot.data(), 0);
                const Region* region = RegionFor(fn);
                if (region && region->allocationBase == imageBase_ && region->protect.find("EXECUTE") != std::string::npos) {
                    counts[fn]++;
                    validSamples++;
                }
            }
            catch (...) { continue; }
        }
        if (counts.empty()) throw std::runtime_error("Could not validate ProcessEvent at vtable index " + HexU64((uint64_t)index));
        auto best = std::max_element(counts.begin(), counts.end(), [](auto& a, auto& b) { return a.second < b.second; });
        double agreement = validSamples > 0 ? (100.0 * best->second / validSamples) : 0.0;
        // ACCURACY: require a strong majority, not just "most common of
        // whatever showed up" - ProcessEvent (or any virtual overridden by
        // very few classes) should agree across the large majority of
        // sampled objects; a low agreement ratio means the vtable index
        // itself is probably wrong (points at some other, less-universally-
        // shared virtual function) even if SOME plurality exists.
        if (agreement < 60.0) {
            throw std::runtime_error("ProcessEvent candidate at vtable index " + HexU64((uint64_t)index) +
                " only has " + std::to_string(agreement) + "% agreement across " +
                std::to_string(validSamples) + " valid samples - too weak to trust");
        }
        evidence.push_back(std::to_string(best->second) + "/" + std::to_string(validSamples) +
            " sampled UObject vtables agree at index " + HexU64((uint64_t)index) +
            " (" + std::to_string((int)agreement) + "%)");
        return best->first;
    }

    uint64_t SelectGWorldByCodeReferences(
        const std::vector<std::tuple<uint64_t, uint64_t, ObjectInfo>>& candidates,
        std::vector<std::string>& evidence) {
        if (candidates.size() < 2 || candidates.size() > 8) {
            throw std::runtime_error("GWorld code-reference tie-break requires 2..8 candidates");
        }
        std::vector<size_t> counts(candidates.size(), 0);
        constexpr uint64_t kChunkSize = 4ULL * 1024 * 1024;
        uint64_t executableBytesScanned = 0;
        for (const Region& region : moduleRegions_) {
            if (region.state != "MEM_COMMIT" ||
                region.protect.find("EXECUTE") == std::string::npos ||
                region.size() < 6) continue;
            for (uint64_t address = region.base; address < region.end;) {
                const uint64_t primary = std::min<uint64_t>(kChunkSize, region.end - address);
                const uint64_t request = std::min<uint64_t>(primary + 8, region.end - address);
                std::vector<uint8_t> bytes;
                try { bytes = mem_.Read(address, (size_t)request); }
                catch (...) { address += primary; continue; }
                executableBytesScanned += primary;
                const size_t scanLimit = std::min<size_t>((size_t)primary, bytes.size());
                for (size_t offset = 0; offset + 6 <= bytes.size() && offset < scanLimit; ++offset) {
                    if (offset != 0 && (bytes[offset - 1] & 0xF0) == 0x40) continue;
                    auto target = TryRipRelativeTargetImpl(
                        bytes.data() + offset, bytes.size() - offset, address + offset);
                    if (!target) continue;
                    for (size_t index = 0; index < candidates.size(); ++index) {
                        if (*target == std::get<0>(candidates[index])) ++counts[index];
                    }
                }
                address += primary;
            }
        }

        size_t winner = candidates.size();
        size_t winningCount = 0;
        for (size_t index = 0; index < candidates.size(); ++index) {
            evidence.push_back("GWorld candidate=" + HexU64(std::get<0>(candidates[index])) +
                " world=" + HexU64(std::get<1>(candidates[index])) +
                " direct_code_refs=" + std::to_string(counts[index]));
            if (counts[index] > winningCount) {
                winner = index;
                winningCount = counts[index];
            }
            else if (counts[index] == winningCount && counts[index] != 0) {
                winner = candidates.size();
            }
        }
        evidence.push_back("ambiguity tie-break executable bytes=" +
            std::to_string(executableBytesScanned));
        if (winner >= candidates.size() || winningCount == 0) {
            std::string detail;
            for (size_t index = 0; index < candidates.size(); ++index) {
                if (!detail.empty()) detail += ", ";
                detail += HexU64(std::get<0>(candidates[index])) +
                    ":refs=" + std::to_string(counts[index]);
            }
            throw std::runtime_error("GWorld candidates remain ambiguous after direct RIP-reference scoring: " + detail);
        }
        evidence.push_back("selected unique direct-code-reference winner refs=" +
            std::to_string(winningCount));
        return std::get<0>(candidates[winner]);
    }

    // --- find_gworld ---
    uint64_t FindGWorld(FNameDecoder& names, const GObjectsInfo& gobjInfo, std::vector<std::string>& evidence) {
        uint32_t chunk0Count = std::min<uint32_t>(ELEMENTS_PER_CHUNK, gobjInfo.numElements);
        auto chunkPointers = mem_.Read(gobjInfo.objects, (size_t)gobjInfo.numChunks * 8);
        const uint64_t firstChunk = u64(chunkPointers.data(), 0);
        auto chunk0 = mem_.Read(firstChunk, (size_t)chunk0Count * FUOBJECT_ITEM_SIZE);
        uint64_t worldClass = 0;
        size_t elemCount = chunk0.size() / FUOBJECT_ITEM_SIZE;
        for (size_t i = 0; i < std::min<size_t>(ELEMENTS_PER_CHUNK, elemCount); i++) {
            uint64_t obj = u64(chunk0.data(), i * FUOBJECT_ITEM_SIZE);
            if (!is_ptr(obj)) continue;
            auto info = GetObjectInfo(obj, names);
            if (!info || info->name != "World") continue;
            auto clsInfo = GetObjectInfo(info->cls, names);
            if (clsInfo && clsInfo->name == "Class") {
                worldClass = obj;
                evidence.push_back("UClass World=" + HexU64(worldClass) + " object_index=" + std::to_string(i));
                break;
            }
        }
        if (!worldClass) throw std::runtime_error("Could not find UClass named World in GObjects chunk0");

        const auto isGObjectsMember = [&](uint64_t object, int32_t internalIndex) {
            if (internalIndex < 0 || (uint32_t)internalIndex >= gobjInfo.numElements) return false;
            const uint32_t chunkIndex = (uint32_t)internalIndex / ELEMENTS_PER_CHUNK;
            const uint32_t itemIndex = (uint32_t)internalIndex % ELEMENTS_PER_CHUNK;
            if (chunkIndex >= gobjInfo.numChunks ||
                (size_t)(chunkIndex + 1) * 8 > chunkPointers.size()) return false;
            const uint64_t chunk = u64(chunkPointers.data(), (size_t)chunkIndex * 8);
            if (!is_ptr(chunk)) return false;
            try {
                auto item = mem_.Read(chunk + (uint64_t)itemIndex * FUOBJECT_ITEM_SIZE, 8);
                return item.size() == 8 && u64(item.data(), 0) == object;
            }
            catch (...) { return false; }
        };
        const auto hasViewportChain = [&](uint64_t world) {
            try {
                const auto gameInstanceBytes = mem_.Read(world + 0x230, 8);
                const uint64_t gameInstance = u64(gameInstanceBytes.data(), 0);
                if (!is_ptr(gameInstance)) return false;
                const auto localPlayers = mem_.Read(gameInstance + 0x38, 16);
                const uint64_t localPlayerData = u64(localPlayers.data(), 0);
                const int32_t count = i32(localPlayers.data(), 8);
                const int32_t capacity = i32(localPlayers.data(), 12);
                if (!is_ptr(localPlayerData) || count < 1 || capacity < count || capacity > 64) return false;
                const auto localPlayerBytes = mem_.Read(localPlayerData, 8);
                const uint64_t localPlayer = u64(localPlayerBytes.data(), 0);
                if (!is_ptr(localPlayer)) return false;
                const auto viewportBytes = mem_.Read(localPlayer + 0x78, 8);
                const uint64_t viewport = u64(viewportBytes.data(), 0);
                if (!is_ptr(viewport)) return false;
                const auto viewportWorldBytes = mem_.Read(viewport + 0x78, 8);
                const auto viewportGameInstanceBytes = mem_.Read(viewport + 0x80, 8);
                const uint64_t viewportWorld = u64(viewportWorldBytes.data(), 0);
                const uint64_t viewportGameInstance = u64(viewportGameInstanceBytes.data(), 0);
                return viewportWorld == world && viewportGameInstance == gameInstance;
            }
            catch (...) { return false; }
        };

        std::vector<std::tuple<uint64_t, uint64_t, ObjectInfo>> refs;
        uint64_t writableBytesScanned = 0;
        for (const Region& region : moduleRegions_) {
            if (region.state != "MEM_COMMIT") continue;
            const bool writable = region.protect.find("READWRITE") != std::string::npos ||
                region.protect.find("WRITECOPY") != std::string::npos ||
                region.protect.find("EXECUTE_READWRITE") != std::string::npos;
            if (!writable || region.size() < 8 || region.size() > 0x2000000) continue;
            std::vector<uint8_t> data;
            try { data = mem_.Read(region.base, (size_t)region.size()); }
            catch (...) { continue; }
            writableBytesScanned += data.size();
            for (size_t off = 0; off + 8 <= data.size(); off += 8) {
                const uint64_t object = u64(data.data(), off);
                if (!is_ptr(object)) continue;
                const Region* objectRegion = RegionFor(object);
                if (!objectRegion || objectRegion->state != "MEM_COMMIT") continue;
                std::vector<uint8_t> raw;
                try { raw = mem_.Read(object, UOBJECT_SIZE); }
                catch (...) { continue; }
                if (raw.size() < UOBJECT_SIZE || u64(raw.data(), 0x10) != worldClass) continue;
                const int32_t internalIndex = i32(raw.data(), 0x0C);
                if (!isGObjectsMember(object, internalIndex) || !hasViewportChain(object)) continue;
                auto info = GetObjectInfo(object, names);
                if (!info || info->cls != worldClass) continue;
                refs.emplace_back(region.base + off, object, *info);
            }
        }
        std::sort(refs.begin(), refs.end(), [](auto& a, auto& b) { return std::get<0>(a) < std::get<0>(b); });
        refs.erase(std::unique(refs.begin(), refs.end(), [](auto& a, auto& b) {
            return std::get<0>(a) == std::get<0>(b);
        }), refs.end());
        if (refs.empty()) {
            throw std::runtime_error("Validated writable-section GWorld scan found 0 candidates");
        }
        if (refs.size() > 1) {
            const uint64_t selected = SelectGWorldByCodeReferences(refs, evidence);
            refs.erase(std::remove_if(refs.begin(), refs.end(), [&](const auto& candidate) {
                return std::get<0>(candidate) != selected;
            }), refs.end());
        }
        auto& [va, ptr, info] = refs.front();
        evidence.push_back("single-pass writable module scan bytes=" +
            std::to_string(writableBytesScanned));
        evidence.push_back("candidate validated by World class, O(1) GObjects membership, and viewport chain");
        evidence.push_back("GWorld points to " + HexU64(ptr) + " name=" + info.name +
            " object_index=" + std::to_string(info.index));
        return va;
    }

    // Resolve UGameViewportClient::Tick from the live viewport vtable instead
    // of carrying a version-specific slot. This is a tiny bounded read (97
    // vtable entries plus at most 0x90 bytes per executable candidate), so it
    // does not add another image scan to the fast semantic path.
    size_t FindViewportTickIndex(uint64_t gworld, std::vector<std::string>& evidence) {
        constexpr size_t kWorldGameInstanceOffset = 0x230;
        constexpr size_t kGameInstanceLocalPlayersOffset = 0x38;
        constexpr size_t kLocalPlayerViewportOffset = 0x78;
        constexpr size_t kViewportWorldOffset = 0x78;
        constexpr size_t kViewportGameInstanceOffset = 0x80;
        constexpr size_t kFirstIndex = 64;
        constexpr size_t kLastIndex = 160;
        constexpr size_t kCodeWindow = 0x90;

        const auto readPointer = [&](uint64_t address) {
            const auto bytes = mem_.Read(address, sizeof(uint64_t));
            if (bytes.size() != sizeof(uint64_t))
                throw std::runtime_error("short pointer read at " + HexU64(address));
            return u64(bytes.data(), 0);
        };
        const uint64_t world = readPointer(gworld);
        const uint64_t gameInstance = readPointer(world + kWorldGameInstanceOffset);
        const auto localPlayers = mem_.Read(gameInstance + kGameInstanceLocalPlayersOffset, 16);
        const uint64_t localPlayerData = u64(localPlayers.data(), 0);
        const int32_t localPlayerCount = i32(localPlayers.data(), 8);
        const int32_t localPlayerCapacity = i32(localPlayers.data(), 12);
        if (!is_ptr(world) || !is_ptr(gameInstance) || !is_ptr(localPlayerData) ||
            localPlayerCount < 1 || localPlayerCount > localPlayerCapacity ||
            localPlayerCapacity > 64)
            throw std::runtime_error("Viewport Tick chain failed before LocalPlayer");
        const uint64_t localPlayer = readPointer(localPlayerData);
        const uint64_t viewport = readPointer(localPlayer + kLocalPlayerViewportOffset);
        const uint64_t viewportWorld = readPointer(viewport + kViewportWorldOffset);
        const uint64_t viewportGameInstance = readPointer(viewport + kViewportGameInstanceOffset);
        if (!is_ptr(localPlayer) || !is_ptr(viewport) || viewportWorld != world ||
            viewportGameInstance != gameInstance)
            throw std::runtime_error("Viewport Tick chain failed reverse-consistency validation");
        const uint64_t vtable = readPointer(viewport);
        if (!is_ptr(vtable))
            throw std::runtime_error("Viewport Tick vtable pointer is invalid");

        const auto entries = mem_.Read(
            vtable + kFirstIndex * sizeof(uint64_t),
            (kLastIndex - kFirstIndex + 1) * sizeof(uint64_t));
        struct Candidate { size_t index; uint64_t function; };
        std::vector<Candidate> candidates;
        for (size_t index = kFirstIndex; index <= kLastIndex; ++index) {
            const uint64_t function = u64(
                entries.data(), (index - kFirstIndex) * sizeof(uint64_t));
            const Region* region = RegionFor(function);
            if (!is_ptr(function) || region == nullptr ||
                region->protect.find("EXECUTE") == std::string::npos)
                continue;
            std::vector<uint8_t> code;
            try { code = mem_.Read(function, kCodeWindow); }
            catch (...) { continue; }
            if (code.size() < kCodeWindow) continue;
            bool hasStackFrame = false;
            for (size_t offset = 0; offset + 4 <= 32; ++offset) {
                if (code[offset] == 0x48 && code[offset + 1] == 0x83 && code[offset + 2] == 0xEC) {
                    hasStackFrame = true;
                    break;
                }
                if (offset + 7 <= 32 && code[offset] == 0x48 && code[offset + 1] == 0x81 &&
                    code[offset + 2] == 0xEC) {
                    hasStackFrame = true;
                    break;
                }
            }
            bool hasViewportVCall = false;
            for (size_t offset = 0; offset + 6 <= code.size(); ++offset) {
                if (code[offset] == 0xFF && (code[offset + 1] & 0xF8) == 0x90 &&
                    code[offset + 2] == 0x80 && code[offset + 3] == 0x01 &&
                    code[offset + 4] == 0x00 && code[offset + 5] == 0x00) {
                    hasViewportVCall = true;
                    break;
                }
            }
            if (hasStackFrame && hasViewportVCall)
                candidates.push_back({ index, function });
        }
        evidence.push_back("viewport=" + HexU64(viewport) + " vtable=" + HexU64(vtable));
        for (const Candidate& candidate : candidates)
            evidence.push_back("Viewport Tick semantic candidate index=" +
                std::to_string(candidate.index) + " function=" + HexU64(candidate.function));
        constexpr size_t kPreferredIndex = 100;
        constexpr size_t kMaxIndexDrift = 8;
        const auto exact = std::find_if(candidates.begin(), candidates.end(), [](const Candidate& candidate) {
            return candidate.index == kPreferredIndex;
        });
        if (exact != candidates.end()) {
            evidence.push_back("selected exact semantic match at long-lived viewport Tick index=100");
            return exact->index;
        }

        size_t bestDistance = static_cast<size_t>(-1);
        size_t bestIndex = 0;
        bool tied = false;
        for (const Candidate& candidate : candidates) {
            const size_t distance = candidate.index > kPreferredIndex
                ? candidate.index - kPreferredIndex
                : kPreferredIndex - candidate.index;
            if (distance > kMaxIndexDrift) continue;
            if (distance < bestDistance) {
                bestDistance = distance;
                bestIndex = candidate.index;
                tied = false;
            }
            else if (distance == bestDistance) {
                tied = true;
            }
        }
        if (bestDistance == static_cast<size_t>(-1) || tied) {
            std::string detail;
            for (const Candidate& candidate : candidates) {
                if (!detail.empty()) detail += ", ";
                detail += "index=" + std::to_string(candidate.index) +
                    " function=" + HexU64(candidate.function);
            }
            throw std::runtime_error("Viewport Tick semantic candidates lack a unique nearest stable-slot winner: " + detail);
        }
        evidence.push_back("selected unique nearest semantic viewport Tick index=" +
            std::to_string(bestIndex) + " drift=" + std::to_string(bestDistance));
        return bestIndex;
    }

    // --- find_append_string ---
    uint64_t FindAppendString(uint64_t fnamePool, std::vector<std::string>& evidence) {
        auto [textRva, textSize, pdataRva, pdataSize] = PeRanges();
        auto pdata = mem_.Read(imageBase_ + pdataRva, pdataSize);
        std::vector<std::pair<uint64_t, uint64_t>> funcs;
        for (size_t off = 0; off + 11 < pdata.size(); off += 12) {
            uint32_t begin = u32(pdata.data(), off), end = u32(pdata.data(), off + 4);
            if (begin && begin < end) funcs.emplace_back(imageBase_ + begin, imageBase_ + end);
        }
        std::sort(funcs.begin(), funcs.end());
        std::vector<uint64_t> starts;
        for (auto& f : funcs) starts.push_back(f.first);

        auto funcFor = [&](uint64_t addr) -> std::optional<std::pair<uint64_t, uint64_t>> {
            auto it = std::upper_bound(starts.begin(), starts.end(), addr);
            if (it == starts.begin()) return std::nullopt;
            size_t idx = (size_t)(it - starts.begin()) - 1;
            if (funcs[idx].first <= addr && addr < funcs[idx].second) return funcs[idx];
            return std::nullopt;
            };

        // Scan .text for RIP-relative instructions referencing the FNamePool.
        // The .text section can be 100+ MB, so split it into overlapping windows
        // (0x10 overlap so a boundary-straddling instruction is fully covered by
        // the next window) and scan them in parallel. funcFor() is read-only
        // (binary-searches the immutable starts/funcs vectors), so it is safe to
        // call concurrently; each worker collects locally and merges once.
        const uint32_t kWindow = 0x400000;
        struct Chunk { uint32_t rva; uint32_t size; };
        std::vector<Chunk> chunks;
        for (uint32_t chunkRva = textRva; chunkRva < textRva + textSize; chunkRva += kWindow) {
            uint32_t chunkSize = std::min<uint32_t>(kWindow + 0x10, textRva + textSize - chunkRva);
            chunks.push_back({ chunkRva, chunkSize });
        }
        std::vector<std::pair<uint64_t, uint64_t>> xrefFuncsVec;
        std::mutex xrefMutex;
        ParallelFor(chunks.size(), [&](size_t ci) {
            const Chunk& ch = chunks[ci];
            std::vector<uint8_t> data;
            try { data = mem_.Read(imageBase_ + ch.rva, ch.size); }
            catch (...) { return; }
            std::vector<std::pair<uint64_t, uint64_t>> local;
            for (size_t i = 0; i + 7 < data.size(); i++) {
                uint8_t b0 = data[i], b1 = data[i + 1], b2 = data[i + 2];
                if (b0 != 0x48 && b0 != 0x4C) continue;
                bool okB2 = (b2 == 0x05 || b2 == 0x0D || b2 == 0x15 || b2 == 0x1D || b2 == 0x25 || b2 == 0x2D || b2 == 0x35 || b2 == 0x3D);
                if (!okB2) continue;
                bool okB1 = (b1 == 0x8D || b1 == 0x8B || b1 == 0x89 || b1 == 0x39 || b1 == 0x3B);
                if (!okB1) continue;
                int32_t disp = i32(data.data(), i + 3);
                uint64_t insn = imageBase_ + ch.rva + i;
                uint64_t target = insn + 7 + (int64_t)disp;
                if (target == fnamePool) {
                    auto fn = funcFor(insn);
                    if (fn) local.push_back(*fn);
                }
            }
            if (!local.empty()) {
                std::lock_guard<std::mutex> lock(xrefMutex);
                xrefFuncsVec.insert(xrefFuncsVec.end(), local.begin(), local.end());
            }
            });
        // Overlapping windows + parallel merge produce duplicates in arbitrary
        // order; sort+unique gives the same deterministic deduped function set
        // the old sequential `seen()` filter did.
        std::sort(xrefFuncsVec.begin(), xrefFuncsVec.end());
        xrefFuncsVec.erase(std::unique(xrefFuncsVec.begin(), xrefFuncsVec.end()), xrefFuncsVec.end());

        std::vector<std::tuple<int, uint64_t, uint64_t, std::vector<std::string>>> scored;
        for (auto& [start, end] : xrefFuncsVec) {
            size_t size = (size_t)std::min<uint64_t>(end - start, 0x800);
            std::vector<uint8_t> code;
            try { code = mem_.Read(start, size); }
            catch (...) { continue; }
            int score = 0;
            std::vector<std::string> notes;
            if (ContainsBytes(code, { 0x8b, 0x19 }) || ContainsBytes(code, { 0x8b, 0x1d })) { score += 2; notes.push_back("reads FName.ComparisonIndex"); }
            if (ContainsBytes(code, { 0x83, 0x7e, 0x04, 0x00 }) || ContainsBytes(code, { 0x83, 0x79, 0x04, 0x00 }) || ContainsBytes(code, { 0x83, 0x7c, 0x24 })) { score += 2; notes.push_back("checks FName.Number"); }
            if (ContainsBytes(code, { 0xba, 0x5f, 0x00, 0x00, 0x00 }) || ContainsBytes(code, { 0xb2, 0x5f })) { score += 4; notes.push_back("appends underscore"); }
            if (ContainsBytes(code, { 0xff, 0xca }) || ContainsBytes(code, { 0xff, 0xc8 })) { score += 2; notes.push_back("uses Number-1"); }
            size_t tailStart = code.size() > 64 ? code.size() - 64 : 0;
            std::vector<uint8_t> tail64(code.begin() + tailStart, code.end());
            if (ContainsBytes(tail64, { 0x48, 0x8b, 0xc7 })) { score -= 3; notes.push_back("looks like FString-returning wrapper"); }
            size_t tail32Start = code.size() > 32 ? code.size() - 32 : 0;
            std::vector<uint8_t> tail32(code.begin() + tail32Start, code.end());
            if (ContainsBytes(tail32, { 0xe9 }) || ContainsBytes(tail32, { 0xff, 0xe0 })) { score += 1; notes.push_back("tail-calls builder append helper"); }
            scored.emplace_back(score, start, end, notes);
        }
        if (scored.empty()) throw std::runtime_error("Could not find AppendString candidates from FNamePool xrefs");
        std::sort(scored.begin(), scored.end(), [](auto& a, auto& b) { return std::get<0>(a) > std::get<0>(b); });
        auto& [score, start, end, notes] = scored.front();
        (void)end;
        if (score < 5) throw std::runtime_error("AppendString candidate score too low (" + std::to_string(score) + ")");
        std::string noteStr;
        for (size_t i = 0; i < notes.size(); i++) { if (i) noteStr += "; "; noteStr += notes[i]; }
        evidence.push_back("candidate score=" + std::to_string(score) + "; " + noteStr);
        return start;
    }

private:
    static bool Contains(const std::vector<uint8_t>& hay, const std::string& needle) {
        if (needle.size() > hay.size()) return false;
        for (size_t i = 0; i + needle.size() <= hay.size(); i++)
            if (memcmp(hay.data() + i, needle.data(), needle.size()) == 0) return true;
        return false;
    }
    static bool ContainsBytes(const std::vector<uint8_t>& hay, std::initializer_list<uint8_t> needle) {
        if (needle.size() == 0 || needle.size() > hay.size()) return false;
        std::vector<uint8_t> n(needle);
        for (size_t i = 0; i + n.size() <= hay.size(); i++)
            if (memcmp(hay.data() + i, n.data(), n.size()) == 0) return true;
        return false;
    }

    uint64_t DetectImageBase() {
        NullLog() << "[.] " << regions_.size() << " memory region(s) enumerated total\n";

        try {
            auto mz = mem_.Read(0x140000000ULL, 2);
            NullLog() << "[.] default base 0x140000000 -> read " << mz.size() << " byte(s)\n";
            if (mz.size() == 2 && mz[0] == 'M' && mz[1] == 'Z') return 0x140000000ULL;
        }
        catch (const std::exception& e) {
            NullLog() << "[.] default base 0x140000000 -> read failed: " << e.what() << "\n";
        }

        throw std::runtime_error("Could not detect module base");
    }

    std::tuple<uint32_t, uint32_t, uint32_t, uint32_t> PeRanges() {
        auto dos = mem_.Read(imageBase_, 0x2000);
        uint32_t eLfanew = u32(dos.data(), 0x3C);
        uint16_t numSections = u16(dos.data(), eLfanew + 6);
        uint16_t optSize = u16(dos.data(), eLfanew + 20);
        uint32_t opt = eLfanew + 24;
        uint32_t exceptionRva = u32(dos.data(), opt + 0x70 + 3 * 8);
        uint32_t exceptionSize = u32(dos.data(), opt + 0x70 + 3 * 8 + 4);
        uint32_t sectionOff = opt + optSize;
        std::optional<uint32_t> textRva, textSize;
        for (int i = 0; i < numSections; i++) {
            uint32_t off = sectionOff + i * 40;
            std::string name((const char*)dos.data() + off, 8);
            name = name.substr(0, name.find('\0'));
            uint32_t virtualSize = u32(dos.data(), off + 8);
            uint32_t virtualAddress = u32(dos.data(), off + 12);
            if (name == ".text") { textRva = virtualAddress; textSize = virtualSize; break; }
        }
        if (!textRva || !textSize) throw std::runtime_error("Could not parse PE .text range");
        return { *textRva, *textSize, exceptionRva, exceptionSize };
    }

    MemoryIO& mem_;
    DWORD pid_;
    std::string processName_;
    std::vector<Region> regions_;
    std::vector<uint64_t> regionBases_;
    std::vector<Region> moduleRegions_;
    uint64_t imageBase_ = 0;
};

void SetError(const char* text, wchar_t* output, size_t capacity) noexcept
{
    if (output == nullptr || capacity == 0) return;
    output[0] = L'\0';
    if (text == nullptr) return;
    const size_t bounded_capacity = capacity > static_cast<size_t>(INT_MAX)
        ? static_cast<size_t>(INT_MAX)
        : capacity;
    const int output_size = static_cast<int>(bounded_capacity);
    if (output_size <= 0) return;
    int written = MultiByteToWideChar(
        CP_UTF8, 0, text, -1, output, output_size);
    if (written == 0) output[0] = L'\0';
    output[bounded_capacity - 1] = L'\0';
}

void SetError(const wchar_t* text, wchar_t* output, size_t capacity) noexcept
{
    if (output == nullptr || capacity == 0) return;
    output[0] = L'\0';
    if (text == nullptr) return;
    size_t index = 0;
    while (index + 1 < capacity && text[index] != L'\0')
    {
        output[index] = text[index];
        ++index;
    }
    output[index] = L'\0';
}

bool ReadImageMetadata(uintptr_t image_base, size_t& image_size)
{
    IMAGE_DOS_HEADER dos{};
    SIZE_T read = 0;
    if (!ReadProcessMemory(GetCurrentProcess(), reinterpret_cast<const void*>(image_base),
            &dos, sizeof(dos), &read) || read != sizeof(dos) ||
        dos.e_magic != IMAGE_DOS_SIGNATURE || dos.e_lfanew <= 0 || dos.e_lfanew > 0x100000)
        return false;
    IMAGE_NT_HEADERS64 nt{};
    if (!ReadProcessMemory(GetCurrentProcess(),
            reinterpret_cast<const void*>(image_base + static_cast<size_t>(dos.e_lfanew)),
            &nt, sizeof(nt), &read) || read != sizeof(nt) ||
        nt.Signature != IMAGE_NT_SIGNATURE ||
        nt.FileHeader.Machine != IMAGE_FILE_MACHINE_AMD64 ||
        nt.OptionalHeader.Magic != IMAGE_NT_OPTIONAL_HDR64_MAGIC ||
        nt.OptionalHeader.SizeOfImage == 0 || nt.OptionalHeader.SizeOfImage > 0x40000000)
        return false;
    image_size = nt.OptionalHeader.SizeOfImage;
    return true;
}
} // namespace

namespace detail
{
bool TryRipRelativeTarget(
    const uint8_t* code,
    size_t size,
    uint64_t instruction_address,
    uint64_t& result) noexcept
{
    const std::optional<uint64_t> target =
        TryRipRelativeTargetImpl(code, size, instruction_address);
    if (!target) return false;
    result = *target;
    return true;
}

bool ValidateResolvedOffsets(
    const ResolvedOffsets& candidate,
    uintptr_t image_base) noexcept
{
    if (image_base == 0 || candidate.image_size == 0 ||
        candidate.image_size > UINTPTR_MAX - image_base ||
        candidate.source != ResolutionSource::FindOffsets ||
        candidate.viewport_tick_index >= 4096 ||
        candidate.process_event_index >= 4096)
        return false;
    const uintptr_t image_end = image_base + candidate.image_size;
    const uintptr_t addresses[]{
        candidate.append_name_address,
        candidate.fname_pool_address,
        candidate.gobjects_address,
        candidate.gworld_address,
        candidate.process_event_address,
    };
    for (uintptr_t address : addresses)
    {
        if (address < image_base || address >= image_end)
            return false;
    }
    return true;
}
} // namespace detail

bool ResolveCurrentProcessNoexcept(
    ResolvedOffsets& result,
    wchar_t* error,
    size_t error_capacity,
    void* cancellation_event) noexcept
{
    result = {};
    if (error != nullptr && error_capacity != 0) error[0] = L'\0';
    try
    {
        const HMODULE module = GetModuleHandleW(nullptr);
        if (module == nullptr)
            throw std::runtime_error("GetModuleHandleW(nullptr) failed");
        const uintptr_t image_base = reinterpret_cast<uintptr_t>(module);
        MemoryIO memory(
            GetCurrentProcessId(),
            GetCurrentProcess(),
            GetCurrentProcess(),
            static_cast<HANDLE>(cancellation_event));
		if (memory.Cancelled())
			throw std::runtime_error("find_offsets cancelled");
        OffsetFinder finder(
            memory,
            GetCurrentProcessId(),
            "current-process",
            std::optional<uint64_t>(image_base));

        std::vector<std::string> evidence;
        const uint64_t fname_pool = finder.FindFNamePool(evidence);
        FNameDecoder names = finder.LoadFNamePool(fname_pool);
        evidence.clear();
        GObjectsInfo gobjects_info{};
        const uint64_t gobjects =
            finder.FindGObjects(names, gobjects_info, evidence);
        evidence.clear();
        constexpr size_t process_event_index = 0x4C;
        const uint64_t process_event = finder.FindProcessEvent(
            gobjects_info, static_cast<int>(process_event_index), evidence);
        evidence.clear();
        const uint64_t gworld =
            finder.FindGWorld(names, gobjects_info, evidence);
        evidence.clear();
        const size_t viewport_tick_index =
            finder.FindViewportTickIndex(gworld, evidence);
        evidence.clear();
        const uint64_t append_name =
            finder.FindAppendString(fname_pool, evidence);

        size_t image_size = 0;
        if (!ReadImageMetadata(image_base, image_size))
            throw std::runtime_error("current executable has invalid PE metadata");

        ResolvedOffsets candidate{
            static_cast<uintptr_t>(append_name),
            static_cast<uintptr_t>(fname_pool),
            static_cast<uintptr_t>(gobjects),
            static_cast<uintptr_t>(gworld),
            static_cast<uintptr_t>(process_event),
            image_size,
            viewport_tick_index,
            process_event_index,
            ResolutionSource::FindOffsets,
        };
        if (!detail::ValidateResolvedOffsets(candidate, image_base))
            throw std::runtime_error("find_offsets returned an invalid offset set");
        result = candidate;
        return true;
    }
    catch (const std::exception& exception)
    {
        SetError(exception.what(), error, error_capacity);
        return false;
    }
    catch (...)
    {
        SetError("unknown find_offsets failure", error, error_capacity);
        return false;
    }
}

int ResolveCurrentProcessGuarded(
    ResolvedOffsets* result,
    wchar_t* error,
    size_t error_capacity,
    void* cancellation_event) noexcept
{
    __try
    {
        return ResolveCurrentProcessNoexcept(
            *result, error, error_capacity, cancellation_event)
            ? 1
            : 0;
    }
    __except (EXCEPTION_EXECUTE_HANDLER)
    {
        wchar_t message[128]{};
        swprintf_s(
            message,
            L"find_offsets stopped on structured exception 0x%08lX",
            static_cast<unsigned long>(GetExceptionCode()));
        *result = {};
        SetError(message, error, error_capacity);
        return 0;
    }
}

bool ResolveCurrentProcess(
    ResolvedOffsets& result,
    wchar_t* error,
    size_t error_capacity,
    void* cancellation_event) noexcept
{
    return ResolveCurrentProcessGuarded(
        &result, error, error_capacity, cancellation_event) == 1;
}
} // namespace nte::mods::offsets::find_offsets
