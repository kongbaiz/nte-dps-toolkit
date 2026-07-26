#pragma once

#include <stdint.h>

#define NTE_EQUIPMENT_GRID_MIN 1
#define NTE_EQUIPMENT_GRID_MAX 5
#define NTE_EQUIPMENT_MAX_PLACEMENTS 64u
#define NTE_COMBAT_CLOCK_HISTORY_SIZE 64u
#define NTE_EQUIPMENT_IPC_VERSION 6u
#define NTE_EQUIPMENT_PIPE_NAME L"\\\\.\\pipe\\nte-equipment-plugin-v6"
#define NTE_EQUIPMENT_IPC_MAGIC 0x5145544Eu
#define NTE_EQUIPMENT_IPC_REQUEST_SIZE 1080u
#define NTE_EQUIPMENT_IPC_RESPONSE_SIZE 2072u
#define NTE_COMBAT_CLOCK_PAUSE_VALID 0x1u

typedef enum NteEquipmentStatus
{
    NTE_EQUIPMENT_STATUS_RPC_DISPATCHED = 0,
    NTE_EQUIPMENT_STATUS_DRY_RUN_OK = 1,
    NTE_EQUIPMENT_STATUS_INVALID_CONTEXT = 2,
    NTE_EQUIPMENT_STATUS_INVALID_MODE = 3,
    NTE_EQUIPMENT_STATUS_INVALID_PLAYER_STATE = 4,
    NTE_EQUIPMENT_STATUS_INVALID_ITEM_ID = 5,
    NTE_EQUIPMENT_STATUS_INVALID_PLACEMENT_BUFFER = 6,
    NTE_EQUIPMENT_STATUS_INVALID_GRID_POSITION = 7,
    NTE_EQUIPMENT_STATUS_TOO_MANY_PLACEMENTS = 8,
    NTE_EQUIPMENT_STATUS_EMPTY_LOADOUT = 9,
    NTE_EQUIPMENT_STATUS_FUNCTION_NOT_FOUND = 10,
    NTE_EQUIPMENT_STATUS_INVALID_IPC_REQUEST = 11,
    NTE_EQUIPMENT_STATUS_INVALID_BOOLEAN_VALUE = 12,
} NteEquipmentStatus;

typedef enum NteEquipmentIpcOperation
{
    NTE_EQUIPMENT_IPC_EQUIP_MODULE = 1,
    NTE_EQUIPMENT_IPC_EQUIP_CORE = 2,
    NTE_EQUIPMENT_IPC_UNEQUIP_MODULE = 3,
    NTE_EQUIPMENT_IPC_UNEQUIP_CORE = 4,
    NTE_EQUIPMENT_IPC_UNEQUIP_ALL = 5,
    NTE_EQUIPMENT_IPC_EQUIP_ONE_KEY = 6,
    NTE_EQUIPMENT_IPC_MOVE_MODULE_TO_CHARACTER = 7,
    NTE_EQUIPMENT_IPC_MOVE_CORE_TO_CHARACTER = 8,
    NTE_EQUIPMENT_IPC_SET_ITEM_DISCARDED = 9,
    NTE_EQUIPMENT_IPC_SET_ITEM_LOCKED = 10,
    NTE_EQUIPMENT_IPC_QUERY_COMBAT_CLOCK_TRANSITIONS = 11,
} NteEquipmentIpcOperation;

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

typedef struct NteEquipmentIpcRequest
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
} NteEquipmentIpcRequest;

typedef struct NteCombatClockTransition
{
    uint64_t sequence;
    uint64_t timestamp_100ns;
    uint32_t pause_type_mask;
    int32_t reserved_value;
    uint32_t state_flags;
    uint32_t reserved;
} NteCombatClockTransition;

typedef struct NteEquipmentIpcResponse
{
    uint32_t magic;
    uint16_t version;
    uint16_t reserved;
    uint64_t request_id;
    uint32_t status;
    uint32_t combat_clock_transition_count;
    NteCombatClockTransition
        combat_clock_transitions[NTE_COMBAT_CLOCK_HISTORY_SIZE];
} NteEquipmentIpcResponse;

#if defined(__cplusplus)
static_assert(sizeof(NteItemNetId) == 8);
static_assert(sizeof(NteEquipmentPlacement) == 16);
static_assert(sizeof(NteCombatClockTransition) == 32);
static_assert(sizeof(NteEquipmentIpcRequest) == NTE_EQUIPMENT_IPC_REQUEST_SIZE);
static_assert(sizeof(NteEquipmentIpcResponse) == NTE_EQUIPMENT_IPC_RESPONSE_SIZE);
#endif
