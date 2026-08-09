#pragma once

#include <stdint.h>

#define NTE_EQUIPMENT_GRID_MIN 1
#define NTE_EQUIPMENT_GRID_MAX 5
#define NTE_EQUIPMENT_MAX_PLACEMENTS 64u
#define NTE_COMBAT_CLOCK_HISTORY_SIZE 64u
#define NTE_MOD_EVENT_HISTORY_SIZE 18u
#define NTE_MOD_EVENT_ID_SIZE 32u
#define NTE_MOD_EVENT_NAME_SIZE 32u
#define NTE_MOD_EVENT_VALUE_COUNT 3u
#define NTE_MOD_LOG_HISTORY_SIZE 18u
#define NTE_MOD_LOG_ID_SIZE 32u
#define NTE_MOD_LOG_MESSAGE_SIZE 56u
#define NTE_MODS_IPC_VERSION 7u
#define NTE_MODS_PIPE_NAME L"\\\\.\\pipe\\nte-mods-plugin-v7"
#define NTE_MODS_RUNTIME_PRESENCE_NAME L"Local\\nte-mods-plugin-v1-present"
#define NTE_MODS_IPC_MAGIC 0x5145544Eu
#define NTE_MODS_IPC_REQUEST_SIZE 1080u
#define NTE_MODS_IPC_RESPONSE_SIZE 2072u
#define NTE_COMBAT_CLOCK_PAUSE_VALID 0x1u

typedef enum NteModsStatus
{
    NTE_MODS_STATUS_RPC_DISPATCHED = 0,
    NTE_MODS_STATUS_DRY_RUN_OK = 1,
    NTE_MODS_STATUS_INVALID_CONTEXT = 2,
    NTE_MODS_STATUS_INVALID_MODE = 3,
    NTE_MODS_STATUS_INVALID_PLAYER_STATE = 4,
    NTE_MODS_STATUS_INVALID_ITEM_ID = 5,
    NTE_MODS_STATUS_INVALID_PLACEMENT_BUFFER = 6,
    NTE_MODS_STATUS_INVALID_GRID_POSITION = 7,
    NTE_MODS_STATUS_TOO_MANY_PLACEMENTS = 8,
    NTE_MODS_STATUS_EMPTY_LOADOUT = 9,
    NTE_MODS_STATUS_FUNCTION_NOT_FOUND = 10,
    NTE_MODS_STATUS_INVALID_IPC_REQUEST = 11,
    NTE_MODS_STATUS_INVALID_BOOLEAN_VALUE = 12,
    NTE_MODS_STATUS_MOD_DISABLED = 13,
} NteModsStatus;

typedef enum NteModsIpcOperation
{
    NTE_MODS_IPC_EQUIP_MODULE = 1,
    NTE_MODS_IPC_EQUIP_CORE = 2,
    NTE_MODS_IPC_UNEQUIP_MODULE = 3,
    NTE_MODS_IPC_UNEQUIP_CORE = 4,
    NTE_MODS_IPC_UNEQUIP_ALL = 5,
    NTE_MODS_IPC_EQUIP_ONE_KEY = 6,
    NTE_MODS_IPC_MOVE_MODULE_TO_CHARACTER = 7,
    NTE_MODS_IPC_MOVE_CORE_TO_CHARACTER = 8,
    NTE_MODS_IPC_SET_ITEM_DISCARDED = 9,
    NTE_MODS_IPC_SET_ITEM_LOCKED = 10,
    NTE_MODS_IPC_QUERY_COMBAT_CLOCK_TRANSITIONS = 11,
    NTE_MODS_IPC_QUERY_MOD_EVENTS = 12,
    NTE_MODS_IPC_QUERY_MOD_LOGS = 13,
} NteModsIpcOperation;

typedef enum NteModLogLevel
{
    NTE_MOD_LOG_INFO = 1,
    NTE_MOD_LOG_WARNING = 2,
    NTE_MOD_LOG_ERROR = 3,
} NteModLogLevel;

typedef struct NteItemNetId
{
    uint32_t slot;
    uint32_t serial;
} NteItemNetId;

typedef struct NteEquipmentPlacement
{
    NteItemNetId equipment;
    int32_t row;
    int32_t column;
} NteEquipmentPlacement;

typedef struct NteModsIpcRequest
{
    uint32_t magic;
    uint16_t version;
    uint16_t operation;
    uint64_t request_id;
    NteItemNetId character;
    NteItemNetId equipment;
    NteItemNetId core;
    int32_t row;
    int32_t column;
    uint32_t placement_count;
    uint32_t state;
    NteEquipmentPlacement placements[NTE_EQUIPMENT_MAX_PLACEMENTS];
} NteModsIpcRequest;

typedef struct NteCombatClockTransition
{
    uint64_t sequence;
    uint64_t timestamp_100ns;
    uint32_t pause_type_mask;
    int32_t reserved_value;
    uint32_t state_flags;
    uint32_t reserved;
} NteCombatClockTransition;

typedef struct NteModEvent
{
    uint64_t sequence;
    uint64_t timestamp_100ns;
    char mod_id[NTE_MOD_EVENT_ID_SIZE];
    char name[NTE_MOD_EVENT_NAME_SIZE];
    uint32_t value_count;
    uint32_t reserved;
    uint64_t values[NTE_MOD_EVENT_VALUE_COUNT];
} NteModEvent;

typedef struct NteModLogEntry
{
    uint64_t sequence;
    uint64_t timestamp_100ns;
    char mod_id[NTE_MOD_LOG_ID_SIZE];
    uint32_t level;
    uint32_t reserved;
    char message[NTE_MOD_LOG_MESSAGE_SIZE];
} NteModLogEntry;

typedef union NteModsIpcPayload
{
    NteCombatClockTransition
        combat_clock_transitions[NTE_COMBAT_CLOCK_HISTORY_SIZE];
    NteModEvent mod_events[NTE_MOD_EVENT_HISTORY_SIZE];
    NteModLogEntry mod_logs[NTE_MOD_LOG_HISTORY_SIZE];
    uint8_t bytes[
        NTE_COMBAT_CLOCK_HISTORY_SIZE * sizeof(NteCombatClockTransition)];
} NteModsIpcPayload;

typedef struct NteModsIpcResponse
{
    uint32_t magic;
    uint16_t version;
    uint16_t reserved;
    uint64_t request_id;
    uint32_t status;
    uint32_t record_count;
    NteModsIpcPayload payload;
} NteModsIpcResponse;

#if defined(__cplusplus)
static_assert(sizeof(NteItemNetId) == 8);
static_assert(sizeof(NteEquipmentPlacement) == 16);
static_assert(sizeof(NteCombatClockTransition) == 32);
static_assert(sizeof(NteModEvent) == 112);
static_assert(sizeof(NteModLogEntry) == 112);
static_assert(sizeof(NteModsIpcPayload) == 2048);
static_assert(sizeof(NteModsIpcRequest) == NTE_MODS_IPC_REQUEST_SIZE);
static_assert(sizeof(NteModsIpcResponse) == NTE_MODS_IPC_RESPONSE_SIZE);
#endif
