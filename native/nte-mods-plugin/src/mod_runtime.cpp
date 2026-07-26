#include "mod_runtime.hpp"

#include "host_api.hpp"
#include "ipc_transport.hpp"
#include "memory_access.hpp"
#include "obfuscated_string.hpp"

#include <Windows.h>

#include <array>
#include <cstddef>
#include <cstdint>

namespace nte::mods::runtime
{
	namespace
	{
		constexpr size_t MAX_MOD_ID_LENGTH = NTE_MOD_EVENT_ID_SIZE - 1;
		constexpr size_t MAX_ENABLED_MODS = 16;
		constexpr size_t MAX_PROGRAM_INSTRUCTIONS = 256;
		constexpr size_t REGISTER_COUNT = 16;
		constexpr size_t MAX_USER_VARIABLES = 12;
		constexpr uint8_t SCRATCH_REGISTER_A = 12;
		constexpr uint8_t SCRATCH_REGISTER_B = 13;
		constexpr uint8_t CONDITION_REGISTER = 14;
		constexpr uint8_t RESULT_REGISTER = 15;
		constexpr size_t MAX_STATE_VARIABLES = 16;
		constexpr size_t MAX_STRING_CONSTANTS = 16;
		constexpr size_t MAX_STRING_LENGTH = NTE_MOD_EVENT_NAME_SIZE - 1;
		constexpr size_t MAX_VARIABLE_NAME_LENGTH = 32;
		constexpr uint8_t NULL_REGISTER = 0xFF;
		constexpr uint64_t MAX_MEMORY_OFFSET = 0x4000;
		constexpr DWORD MAX_SCRIPT_BYTES = 16 * 1024;
		constexpr size_t VIEWPORT_GAME_INSTANCE_OFFSET = 0x80;
		constexpr size_t GAME_INSTANCE_LOCAL_PLAYERS_OFFSET = 0x38;
		constexpr size_t LOCAL_PLAYER_CONTROLLER_OFFSET = 0x30;

		struct TextView
		{
			const char* data;
			size_t size;
		};

		struct EnabledMod
		{
			std::array<char, MAX_MOD_ID_LENGTH + 1> id;
		};

		struct EnabledModSet
		{
			std::array<EnabledMod, MAX_ENABLED_MODS> mods;
			size_t count;
		};

		enum class OpCode : uint8_t
		{
			LoadImmediate,
			LoadEventViewport,
			LoadGameValue,
			Move,
			LoadState,
			StoreState,
			Add,
			Subtract,
			Multiply,
			Divide,
			Modulo,
			BitAnd,
			BitOr,
			BitXor,
			ShiftLeft,
			ShiftRight,
			Equal,
			NotEqual,
			Less,
			LessEqual,
			Greater,
			GreaterEqual,
			LogicalAnd,
			LogicalOr,
			LogicalNot,
			Negate,
			ReadPointer,
			ReadU8,
			ReadU16,
			ReadU32,
			ReadU64,
			ReadI32,
			ReadPointerArrayFirst,
			ReadPointerArrayCount,
			IsReadable,
			TickMilliseconds,
			EquipmentCacheMissing,
			EquipmentCacheReady,
			InvokeSdkRead,
			SampleCombatClock,
			ReadCombatClockPauseMask,
			ReadCombatClockStateFlags,
			JumpIfFalse,
			JumpIfLoopDone,
			Jump,
			LoopNext,
			PrepareEquipment,
			ForwardCombatClock,
			BindIpcContext,
			EmitIpcEvent,
			DebugLog,
		};

		struct Instruction
		{
			OpCode opcode;
			uint8_t first;
			uint8_t second;
			uint8_t third;
			uint8_t fourth;
			uint8_t reserved[3];
			uint64_t immediate;
		};

		struct NamedValue
		{
			std::array<char, MAX_VARIABLE_NAME_LENGTH + 1> name;
			uint8_t length;
			uint64_t value;
		};

		struct StringConstant
		{
			std::array<char, MAX_STRING_LENGTH + 1> value;
			uint8_t length;
		};

		struct ModProgram
		{
			EnabledMod mod;
			uint32_t capabilities;
			uint32_t used_capabilities;
			std::array<Instruction, MAX_PROGRAM_INSTRUCTIONS> instructions;
			size_t instruction_count;
			std::array<NamedValue, MAX_STATE_VARIABLES> states;
			size_t state_count;
			std::array<StringConstant, MAX_STRING_CONSTANTS> strings;
			size_t string_count;
		};

		struct Variable
		{
			std::array<char, MAX_VARIABLE_NAME_LENGTH + 1> name;
			uint8_t length;
			uint8_t register_index;
		};

		struct VariableSet
		{
			std::array<Variable, MAX_USER_VARIABLES> variables;
			size_t count;
		};

		struct PointerArray
		{
			void** data;
			int32_t count;
			int32_t capacity;
		};

		struct TickExecution
		{
			PluginContext ipc_context;
			void* viewport;
			void* game_instance;
			void* local_player;
			void* player_controller;
			void* player_state;
			void* player_character;
			void* sampled_player_controller;
			uint64_t combat_clock_sample;
			bool session_resolved;
			bool player_state_resolved;
			bool player_character_resolved;
			bool combat_clock_sample_resolved;
		};

		enum class GameValue : uint8_t
		{
			Viewport,
			GameInstance,
			LocalPlayer,
			PlayerController,
			PlayerState,
			PlayerCharacter,
		};

		uint32_t enabled_capabilities = 0;
		size_t program_count = 0;
		uint64_t source_fingerprint = 0;
		SRWLOCK program_lock = SRWLOCK_INIT;
		constinit std::array<ModProgram, MAX_ENABLED_MODS> programs{};
		constinit std::array<ModProgram, MAX_ENABLED_MODS> candidate_programs{};
		constinit ModProgram candidate_program{};
		constinit std::array<wchar_t, MAX_PATH> script_path{};
		constinit std::array<char, MAX_SCRIPT_BYTES + 1> script_text{};
		constinit EnabledModSet enabled_mod_set{};
		constinit std::array<NteModEvent, NTE_MOD_EVENT_HISTORY_SIZE>
			mod_event_history{};
		constinit uint32_t mod_event_history_count = 0;
		constinit uint32_t mod_event_history_next = 0;
		constinit uint64_t next_mod_event_sequence = 1;

		static_assert(sizeof(Instruction) == 16);
		static_assert(sizeof(PointerArray) == 16);

		void DebugLog(const wchar_t* message)
		{
		#if defined(_DEBUG)
			OutputDebugStringW(message);
		#else
			static_cast<void>(message);
		#endif
		}

		void DebugLog(const char* message)
		{
		#if defined(_DEBUG)
			OutputDebugStringA(message);
			OutputDebugStringA("\n");
		#else
			static_cast<void>(message);
		#endif
		}

		uint64_t UpdateFingerprint(
			uint64_t fingerprint,
			const void* bytes,
			size_t size)
		{
			const auto* values = static_cast<const uint8_t*>(bytes);
			for (size_t index = 0; index < size; ++index)
			{
				fingerprint ^= values[index];
				fingerprint *= 0x00000100000001B3ull;
			}
			fingerprint ^= 0xFF;
			return fingerprint * 0x00000100000001B3ull;
		}

		bool IsWhitespace(char value)
		{
			return value == ' ' || value == '\t';
		}

		TextView Trim(TextView text)
		{
			while (text.size != 0 && IsWhitespace(*text.data))
			{
				++text.data;
				--text.size;
			}
			while (text.size != 0 && IsWhitespace(text.data[text.size - 1]))
				--text.size;
			return text;
		}

		bool Equals(TextView text, const char* expected)
		{
			size_t index = 0;
			while (index < text.size && expected[index] != '\0')
			{
				if (text.data[index] != expected[index])
					return false;
				++index;
			}
			return index == text.size && expected[index] == '\0';
		}

		bool StartsWith(TextView text, const char* expected)
		{
			for (size_t index = 0; expected[index] != '\0'; ++index)
			{
				if (index == text.size || text.data[index] != expected[index])
					return false;
			}
			return true;
		}

		bool IsValidModId(TextView text)
		{
			if (text.size == 0 || text.size > MAX_MOD_ID_LENGTH)
				return false;
			for (size_t index = 0; index < text.size; ++index)
			{
				const char value = text.data[index];
				if ((value < 'a' || value > 'z') &&
					(value < '0' || value > '9') &&
					value != '-' && value != '_' && value != '.')
					return false;
			}
			return true;
		}

		bool NextToken(TextView& line, TextView& token)
		{
			line = Trim(line);
			if (line.size == 0)
				return false;

			size_t length = 0;
			while (length < line.size && !IsWhitespace(line.data[length]))
				++length;
			token = { line.data, length };
			line.data += length;
			line.size -= length;
			return true;
		}

		bool TokenizeLine(
			TextView line,
			TextView* tokens,
			size_t capacity,
			size_t& count)
		{
			count = 0;
			TextView token{};
			while (NextToken(line, token))
			{
				if (count == capacity)
					return false;
				tokens[count++] = token;
			}
			return true;
		}

		bool ReadLine(TextView& text, TextView& line)
		{
			if (text.size == 0)
				return false;

			size_t length = 0;
			while (length < text.size &&
				text.data[length] != '\r' && text.data[length] != '\n')
				++length;
			line = Trim({ text.data, length });

			size_t consumed = length;
			if (consumed < text.size && text.data[consumed] == '\r')
				++consumed;
			if (consumed < text.size && text.data[consumed] == '\n')
				++consumed;
			text.data += consumed;
			text.size -= consumed;
			return true;
		}

		bool ReadRawLine(TextView& text, TextView& line)
		{
			if (text.size == 0)
				return false;

			size_t length = 0;
			while (length < text.size &&
				text.data[length] != '\r' && text.data[length] != '\n')
				++length;
			line = { text.data, length };

			size_t consumed = length;
			if (consumed < text.size && text.data[consumed] == '\r')
				++consumed;
			if (consumed < text.size && text.data[consumed] == '\n')
				++consumed;
			text.data += consumed;
			text.size -= consumed;
			return true;
		}

		TextView TrimEnd(TextView text)
		{
			while (text.size != 0 && IsWhitespace(text.data[text.size - 1]))
				--text.size;
			return text;
		}

		bool IsIgnoredLine(TextView line)
		{
			return line.size == 0 || line.data[0] == '#';
		}

		TextView StripUtf8Bom(TextView text)
		{
			if (text.size >= 3 &&
				static_cast<uint8_t>(text.data[0]) == 0xEF &&
				static_cast<uint8_t>(text.data[1]) == 0xBB &&
				static_cast<uint8_t>(text.data[2]) == 0xBF)
				return { text.data + 3, text.size - 3 };
			return text;
		}

		bool NextContentLine(TextView& text, TextView& line)
		{
			while (ReadLine(text, line))
			{
				if (!IsIgnoredLine(line))
					return true;
			}
			return false;
		}

		bool NextIndentedContentLine(
			TextView& text,
			TextView& line,
			size_t& indentation)
		{
			TextView raw{};
			while (ReadRawLine(text, raw))
			{
				indentation = 0;
				while (indentation < raw.size &&
					raw.data[indentation] == ' ')
					++indentation;
				line = TrimEnd({
					raw.data + indentation,
					raw.size - indentation,
				});
				if (line.size == 0 || line.data[0] == '#')
					continue;
				return true;
			}
			return false;
		}

		bool CopyModId(TextView id, EnabledMod& output)
		{
			if (!IsValidModId(id))
				return false;
			for (size_t index = 0; index < id.size; ++index)
				output.id[index] = id.data[index];
			output.id[id.size] = '\0';
			return true;
		}

		bool SameModId(const EnabledMod& left, const EnabledMod& right)
		{
			for (size_t index = 0; index <= MAX_MOD_ID_LENGTH; ++index)
			{
				if (left.id[index] != right.id[index])
					return false;
				if (left.id[index] == '\0')
					return true;
			}
			return true;
		}

		bool ParseEnabledModSet(TextView text, EnabledModSet& output)
		{
			TextView line{};
			std::array<TextView, 3> tokens{};
			size_t token_count = 0;
			if (!NextContentLine(text, line) ||
				!TokenizeLine(line, tokens.data(), tokens.size(), token_count) ||
				token_count != 2 || !Equals(tokens[0], "nte_mod_set") ||
				!Equals(tokens[1], "1"))
				return false;

			while (NextContentLine(text, line))
			{
				if (!TokenizeLine(
					line, tokens.data(), tokens.size(), token_count) ||
					token_count != 2 || !Equals(tokens[0], "load") ||
					output.count == MAX_ENABLED_MODS)
					return false;

				EnabledMod candidate{};
				if (!CopyModId(tokens[1], candidate))
					return false;
				for (size_t index = 0; index < output.count; ++index)
				{
					if (SameModId(output.mods[index], candidate))
						return false;
				}
				output.mods[output.count++] = candidate;
			}
			return true;
		}

		uint32_t CapabilityFromName(TextView name)
		{
			if (Equals(name, "viewport.tick"))
				return CAPABILITY_VIEWPORT_TICK;
			if (Equals(name, "memory.read"))
				return CAPABILITY_MEMORY_READ;
			if (Equals(name, "ipc"))
				return CAPABILITY_IPC;
			if (Equals(name, "sdk.read"))
				return CAPABILITY_SDK_READ;
			if (Equals(name, "equipment"))
				return CAPABILITY_EQUIPMENT;
			if (Equals(name, "combat-clock"))
				return CAPABILITY_COMBAT_CLOCK;
			if (Equals(name, "log"))
				return CAPABILITY_LOG;
			if (Equals(name, "game.session"))
				return CAPABILITY_GAME_SESSION;
			return 0;
		}

		bool ParseCall(
			TextView expression,
			const char* function_name,
			TextView& arguments)
		{
			size_t name_length = 0;
			while (function_name[name_length] != '\0')
				++name_length;
			if (expression.size < name_length + 2 ||
				expression.data[name_length] != '(' ||
				expression.data[expression.size - 1] != ')')
				return false;
			for (size_t index = 0; index < name_length; ++index)
			{
				if (expression.data[index] != function_name[index])
					return false;
			}
			arguments = Trim({
				expression.data + name_length + 1,
				expression.size - name_length - 2,
			});
			return true;
		}

		bool ParseStringLiteral(TextView text, TextView& value)
		{
			if (text.size < 2 || text.data[0] != '"' ||
				text.data[text.size - 1] != '"')
				return false;
			value = { text.data + 1, text.size - 2 };
			for (size_t index = 0; index < value.size; ++index)
			{
				if (value.data[index] == '"' || value.data[index] == '\\')
					return false;
			}
			return true;
		}

		bool SplitTwoArguments(
			TextView arguments,
			TextView& first,
			TextView& second)
		{
			size_t separator = arguments.size;
			for (size_t index = 0; index < arguments.size; ++index)
			{
				if (arguments.data[index] != ',')
					continue;
				if (separator != arguments.size)
					return false;
				separator = index;
			}
			if (separator == arguments.size)
				return false;
			first = Trim({ arguments.data, separator });
			second = Trim({
				arguments.data + separator + 1,
				arguments.size - separator - 1,
			});
			return first.size != 0 && second.size != 0;
		}

		bool ParseAssignment(
			TextView line,
			TextView& target,
			TextView& expression)
		{
			size_t separator = line.size;
			for (size_t index = 0; index < line.size; ++index)
			{
				if (line.data[index] != '=')
					continue;
				const bool comparison =
					(index != 0 &&
						(line.data[index - 1] == '=' ||
							line.data[index - 1] == '!' ||
							line.data[index - 1] == '<' ||
							line.data[index - 1] == '>')) ||
					(index + 1 < line.size && line.data[index + 1] == '=');
				if (comparison)
					continue;
				if (separator != line.size)
					return false;
				separator = index;
			}
			if (separator == line.size)
				return false;
			target = Trim({ line.data, separator });
			expression = Trim({
				line.data + separator + 1,
				line.size - separator - 1,
			});
			return target.size != 0 && expression.size != 0;
		}

		bool IsValidVariableName(TextView name)
		{
			if (name.size == 0 || name.size > MAX_VARIABLE_NAME_LENGTH ||
				((name.data[0] < 'a' || name.data[0] > 'z') &&
					name.data[0] != '_'))
				return false;
			for (size_t index = 1; index < name.size; ++index)
			{
				const char value = name.data[index];
				if ((value < 'a' || value > 'z') &&
					(value < '0' || value > '9') &&
					value != '_')
					return false;
			}
			return !Equals(name, "event") && !Equals(name, "state") &&
				!Equals(name, "None") && !Equals(name, "True") &&
				!Equals(name, "False");
		}

		bool FindVariable(
			const VariableSet& variables,
			TextView name,
			uint8_t& register_index)
		{
			for (size_t index = 0; index < variables.count; ++index)
			{
				const Variable& variable = variables.variables[index];
				if (variable.length != name.size)
					continue;
				bool equal = true;
				for (size_t character = 0; character < name.size; ++character)
				{
					if (variable.name[character] != name.data[character])
					{
						equal = false;
						break;
					}
				}
				if (equal)
				{
					register_index = variable.register_index;
					return true;
				}
			}
			return false;
		}

		bool AssignVariable(
			VariableSet& variables,
			TextView name,
			uint8_t& register_index)
		{
			if (!IsValidVariableName(name))
				return false;
			if (FindVariable(variables, name, register_index))
				return true;
			if (variables.count == variables.variables.size())
				return false;

			Variable& variable = variables.variables[variables.count];
			for (size_t index = 0; index < name.size; ++index)
				variable.name[index] = name.data[index];
			variable.name[name.size] = '\0';
			variable.length = static_cast<uint8_t>(name.size);
			variable.register_index = static_cast<uint8_t>(variables.count);
			register_index = variable.register_index;
			++variables.count;
			return true;
		}

		bool ParseNullableVariable(
			const VariableSet& variables,
			TextView name,
			uint8_t& register_index)
		{
			if (Equals(name, "None"))
			{
				register_index = NULL_REGISTER;
				return true;
			}
			return FindVariable(variables, name, register_index);
		}

		bool ParseInteger(TextView text, uint64_t& output)
		{
			if (Equals(text, "None") || Equals(text, "False"))
			{
				output = 0;
				return true;
			}
			if (Equals(text, "True"))
			{
				output = 1;
				return true;
			}
			if (text.size == 0)
				return false;
			uint32_t base = 10;
			size_t index = 0;
			if (text.size > 2 && text.data[0] == '0' &&
				(text.data[1] == 'x' || text.data[1] == 'X'))
			{
				base = 16;
				index = 2;
			}
			uint64_t value = 0;
			for (; index < text.size; ++index)
			{
				const char digit = text.data[index];
				uint32_t parsed = 0;
				if (digit >= '0' && digit <= '9')
					parsed = static_cast<uint32_t>(digit - '0');
				else if (base == 16 && digit >= 'a' && digit <= 'f')
					parsed = static_cast<uint32_t>(digit - 'a' + 10);
				else if (base == 16 && digit >= 'A' && digit <= 'F')
					parsed = static_cast<uint32_t>(digit - 'A' + 10);
				else
					return false;
				if (parsed >= base || value > (UINT64_MAX - parsed) / base)
					return false;
				value = value * base + parsed;
			}
			output = value;
			return true;
		}

		bool ParseStateName(TextView text, TextView& name)
		{
			if (!StartsWith(text, "state.") || text.size <= 6)
				return false;
			name = { text.data + 6, text.size - 6 };
			return IsValidVariableName(name);
		}

		bool FindState(
			const ModProgram& program,
			TextView name,
			uint8_t& index)
		{
			for (size_t state_index = 0;
				state_index < program.state_count;
				++state_index)
			{
				const NamedValue& state = program.states[state_index];
				if (state.length != name.size)
					continue;
				bool same = true;
				for (size_t character = 0; character < name.size; ++character)
				{
					if (state.name[character] != name.data[character])
					{
						same = false;
						break;
					}
				}
				if (same)
				{
					index = static_cast<uint8_t>(state_index);
					return true;
				}
			}
			return false;
		}

		bool AddState(
			ModProgram& program,
			TextView name,
			uint64_t initial_value)
		{
			uint8_t existing = 0;
			if (!IsValidVariableName(name) ||
				FindState(program, name, existing) ||
				program.state_count == program.states.size())
				return false;
			NamedValue& state = program.states[program.state_count];
			for (size_t index = 0; index < name.size; ++index)
				state.name[index] = name.data[index];
			state.name[name.size] = '\0';
			state.length = static_cast<uint8_t>(name.size);
			state.value = initial_value;
			++program.state_count;
			return true;
		}

		bool AddStringConstant(
			ModProgram& program,
			TextView value,
			uint8_t& index)
		{
			if (value.size == 0 || value.size > MAX_STRING_LENGTH)
				return false;
			for (size_t existing = 0;
				existing < program.string_count;
				++existing)
			{
				const StringConstant& candidate = program.strings[existing];
				if (candidate.length != value.size)
					continue;
				bool same = true;
				for (size_t character = 0; character < value.size; ++character)
				{
					if (candidate.value[character] != value.data[character])
					{
						same = false;
						break;
					}
				}
				if (same)
				{
					index = static_cast<uint8_t>(existing);
					return true;
				}
			}
			if (program.string_count == program.strings.size())
				return false;
			StringConstant& target = program.strings[program.string_count];
			for (size_t character = 0; character < value.size; ++character)
				target.value[character] = value.data[character];
			target.value[value.size] = '\0';
			target.length = static_cast<uint8_t>(value.size);
			index = static_cast<uint8_t>(program.string_count++);
			return true;
		}

		bool IsValidEventName(TextView text)
		{
			if (text.size == 0 || text.size > MAX_STRING_LENGTH)
				return false;
			for (size_t index = 0; index < text.size; ++index)
			{
				const char value = text.data[index];
				if ((value < 'a' || value > 'z') &&
					(value < '0' || value > '9') && value != '-' &&
					value != '_' && value != '.')
					return false;
			}
			return true;
		}

		bool AppendInstruction(
			ModProgram& program,
			OpCode opcode,
			uint8_t first = 0,
			uint8_t second = 0,
			uint8_t third = 0,
			uint8_t fourth = 0,
			uint64_t immediate = 0)
		{
			if (program.instruction_count == program.instructions.size())
				return false;
			program.instructions[program.instruction_count++] = {
				opcode,
				first,
				second,
				third,
				fourth,
				{},
				immediate,
			};
			return true;
		}

		bool CompileAtom(
			TextView expression,
			ModProgram& program,
			const VariableSet& variables,
			uint8_t destination)
		{
			expression = Trim(expression);
			uint64_t literal = 0;
			if (ParseInteger(expression, literal))
				return AppendInstruction(
					program,
					OpCode::LoadImmediate,
					destination,
					0,
					0,
					0,
					literal);
			if (Equals(expression, "event.viewport"))
			{
				program.used_capabilities |= CAPABILITY_VIEWPORT_TICK;
				return AppendInstruction(
					program, OpCode::LoadEventViewport, destination);
			}
			GameValue game_value{};
			bool is_game_value = true;
			if (Equals(expression, "game.viewport"))
				game_value = GameValue::Viewport;
			else if (Equals(expression, "game.instance"))
				game_value = GameValue::GameInstance;
			else if (Equals(expression, "game.local_player"))
				game_value = GameValue::LocalPlayer;
			else if (Equals(expression, "game.player_controller"))
				game_value = GameValue::PlayerController;
			else if (Equals(expression, "game.player_state"))
				game_value = GameValue::PlayerState;
			else if (Equals(expression, "game.player_character"))
				game_value = GameValue::PlayerCharacter;
			else
				is_game_value = false;
			if (is_game_value)
			{
				program.used_capabilities |= CAPABILITY_GAME_SESSION;
				return AppendInstruction(
					program,
					OpCode::LoadGameValue,
					destination,
					0,
					0,
					0,
					static_cast<uint64_t>(game_value));
			}
			TextView state_name{};
			uint8_t source = 0;
			if (ParseStateName(expression, state_name))
			{
				return FindState(program, state_name, source) &&
					AppendInstruction(
						program,
						OpCode::LoadState,
						destination,
						0,
						0,
						0,
						source);
			}
			return FindVariable(variables, expression, source) &&
				AppendInstruction(
					program, OpCode::Move, destination, source);
		}

		bool MaterializeAtom(
			TextView expression,
			ModProgram& program,
			const VariableSet& variables,
			uint8_t scratch,
			uint8_t& output)
		{
			expression = Trim(expression);
			if (FindVariable(variables, expression, output))
				return true;
			output = scratch;
			return CompileAtom(expression, program, variables, scratch);
		}

		bool FindBinaryOperator(
			TextView expression,
			TextView& left,
			TextView& right,
			OpCode& opcode)
		{
			struct Candidate
			{
				const char* token;
				size_t length;
				OpCode opcode;
			};
			constexpr std::array<Candidate, 18> candidates{
				Candidate{ " or ", 4, OpCode::LogicalOr },
				Candidate{ " and ", 5, OpCode::LogicalAnd },
				Candidate{ "==", 2, OpCode::Equal },
				Candidate{ "!=", 2, OpCode::NotEqual },
				Candidate{ "<=", 2, OpCode::LessEqual },
				Candidate{ ">=", 2, OpCode::GreaterEqual },
				Candidate{ "<<", 2, OpCode::ShiftLeft },
				Candidate{ ">>", 2, OpCode::ShiftRight },
				Candidate{ "<", 1, OpCode::Less },
				Candidate{ ">", 1, OpCode::Greater },
				Candidate{ "+", 1, OpCode::Add },
				Candidate{ "-", 1, OpCode::Subtract },
				Candidate{ "*", 1, OpCode::Multiply },
				Candidate{ "/", 1, OpCode::Divide },
				Candidate{ "%", 1, OpCode::Modulo },
				Candidate{ "&", 1, OpCode::BitAnd },
				Candidate{ "|", 1, OpCode::BitOr },
				Candidate{ "^", 1, OpCode::BitXor },
			};
			const Candidate* found = nullptr;
			size_t found_at = 0;
			size_t depth = 0;
			bool in_string = false;
			for (size_t index = 0; index < expression.size; ++index)
			{
				const char value = expression.data[index];
				if (value == '"')
					in_string = !in_string;
				else if (!in_string && value == '(')
					++depth;
				else if (!in_string && value == ')')
				{
					if (depth == 0)
						return false;
					--depth;
				}
				if (in_string || depth != 0)
					continue;
				for (const Candidate& candidate : candidates)
				{
					if (index + candidate.length > expression.size)
						continue;
					bool same = true;
					for (size_t character = 0;
						character < candidate.length;
						++character)
					{
						if (expression.data[index + character] !=
							candidate.token[character])
						{
							same = false;
							break;
						}
					}
					if (!same)
						continue;
					if (found != nullptr)
						return false;
					found = &candidate;
					found_at = index;
					index += candidate.length - 1;
					break;
				}
			}
			if (found == nullptr || in_string || depth != 0)
				return false;
			left = Trim({ expression.data, found_at });
			right = Trim({
				expression.data + found_at + found->length,
				expression.size - found_at - found->length,
			});
			opcode = found->opcode;
			return left.size != 0 && right.size != 0;
		}

		bool SplitArguments(
			TextView arguments,
			TextView* output,
			size_t capacity,
			size_t& count)
		{
			count = 0;
			if (arguments.size == 0)
				return true;
			bool in_string = false;
			size_t depth = 0;
			size_t first = 0;
			for (size_t index = 0; index <= arguments.size; ++index)
			{
				const char value = index < arguments.size
					? arguments.data[index]
					: ',';
				if (value == '"')
					in_string = !in_string;
				else if (!in_string && value == '(')
					++depth;
				else if (!in_string && value == ')')
				{
					if (depth == 0)
						return false;
					--depth;
				}
				else if (!in_string && depth == 0 && value == ',')
				{
					if (count == capacity)
						return false;
					TextView argument = Trim({
						arguments.data + first,
						index - first,
					});
					if (argument.size == 0)
						return false;
					output[count++] = argument;
					first = index + 1;
				}
			}
			return !in_string && depth == 0;
		}

		bool ParseSdkCall(
			TextView expression,
			SdkReadApi& api,
			TextView& arguments,
			size_t& argument_count)
		{
			argument_count = 1;
			if (ParseCall(expression, "sdk.player_character", arguments))
				api = SdkReadApi::PlayerCharacter;
			else if (ParseCall(expression, "sdk.player_state", arguments))
				api = SdkReadApi::PlayerState;
			else if (ParseCall(expression, "sdk.game_paused", arguments))
				api = SdkReadApi::GamePaused;
			else if (ParseCall(expression, "sdk.attack_target", arguments))
				api = SdkReadApi::AttackTarget;
			else if (ParseCall(expression, "sdk.current_weapon", arguments))
				api = SdkReadApi::CurrentWeapon;
			else if (ParseCall(expression, "sdk.character_level", arguments))
				api = SdkReadApi::CharacterLevel;
			else if (ParseCall(expression, "sdk.character_hp_milli", arguments))
				api = SdkReadApi::CharacterHpMilli;
			else if (ParseCall(
				expression, "sdk.character_hp_max_milli", arguments))
			{
				api = SdkReadApi::CharacterHpMaxMilli;
				argument_count = 2;
			}
			else if (ParseCall(
				expression, "sdk.character_is_alive", arguments))
				api = SdkReadApi::CharacterIsAlive;
			else if (ParseCall(
				expression, "sdk.character_is_dead", arguments))
				api = SdkReadApi::CharacterIsDead;
			else if (ParseCall(
				expression, "sdk.character_is_controlled", arguments))
				api = SdkReadApi::CharacterIsControlled;
			else if (ParseCall(
				expression, "sdk.character_slomo_milli", arguments))
				api = SdkReadApi::CharacterSlomoMilli;
			else
				return false;
			return true;
		}

		bool CompileCallExpression(
			TextView expression,
			ModProgram& program,
			const VariableSet& variables,
			uint8_t destination)
		{
			TextView arguments{};
			std::array<TextView, 3> parsed{};
			size_t count = 0;
			if (ParseCall(expression, "time.now_ms", arguments))
			{
				return arguments.size == 0 && AppendInstruction(
					program, OpCode::TickMilliseconds, destination);
			}
			if (ParseCall(expression, "equipment.cache_missing", arguments))
			{
				program.used_capabilities |= CAPABILITY_EQUIPMENT;
				return arguments.size == 0 && AppendInstruction(
					program, OpCode::EquipmentCacheMissing, destination);
			}
			if (ParseCall(expression, "equipment.cache_ready", arguments))
			{
				if (!SplitArguments(arguments, parsed.data(), parsed.size(), count) ||
					count != 1)
					return false;
				uint8_t player_state = 0;
				if (!MaterializeAtom(
						parsed[0],
						program,
						variables,
						SCRATCH_REGISTER_A,
						player_state))
					return false;
				program.used_capabilities |= CAPABILITY_EQUIPMENT;
				return AppendInstruction(
					program,
					OpCode::EquipmentCacheReady,
					destination,
					player_state);
			}
			if (ParseCall(expression, "combat_clock.sample", arguments))
			{
				if (!SplitArguments(arguments, parsed.data(), parsed.size(), count) ||
					count != 1)
					return false;
				uint8_t player_controller = 0;
				if (!MaterializeAtom(
						parsed[0],
						program,
						variables,
						SCRATCH_REGISTER_A,
						player_controller))
					return false;
				program.used_capabilities |= CAPABILITY_COMBAT_CLOCK;
				return AppendInstruction(
					program,
					OpCode::SampleCombatClock,
					destination,
					player_controller);
			}
			if (ParseCall(expression, "combat_clock.pause_mask", arguments) ||
				ParseCall(expression, "combat_clock.state_flags", arguments))
			{
				const bool read_state_flags = StartsWith(
					expression, "combat_clock.state_flags");
				if (!SplitArguments(arguments, parsed.data(), parsed.size(), count) ||
					count != 1)
					return false;
				uint8_t player_controller = 0;
				if (!MaterializeAtom(
						parsed[0],
						program,
						variables,
						SCRATCH_REGISTER_A,
						player_controller))
					return false;
				program.used_capabilities |= CAPABILITY_COMBAT_CLOCK;
				return AppendInstruction(
					program,
					read_state_flags
						? OpCode::ReadCombatClockStateFlags
						: OpCode::ReadCombatClockPauseMask,
					destination,
					player_controller);
			}

			OpCode memory_opcode{};
			bool memory_call = true;
			if (ParseCall(expression, "memory.read_ptr", arguments))
				memory_opcode = OpCode::ReadPointer;
			else if (ParseCall(expression, "memory.read_u8", arguments))
				memory_opcode = OpCode::ReadU8;
			else if (ParseCall(expression, "memory.read_u16", arguments))
				memory_opcode = OpCode::ReadU16;
			else if (ParseCall(expression, "memory.read_u32", arguments))
				memory_opcode = OpCode::ReadU32;
			else if (ParseCall(expression, "memory.read_u64", arguments))
				memory_opcode = OpCode::ReadU64;
			else if (ParseCall(expression, "memory.read_i32", arguments))
				memory_opcode = OpCode::ReadI32;
			else if (ParseCall(expression, "memory.tarray_first", arguments))
				memory_opcode = OpCode::ReadPointerArrayFirst;
			else if (ParseCall(expression, "memory.tarray_count", arguments))
				memory_opcode = OpCode::ReadPointerArrayCount;
			else if (ParseCall(expression, "memory.is_readable", arguments))
				memory_opcode = OpCode::IsReadable;
			else
				memory_call = false;
			if (memory_call)
			{
				if (!SplitArguments(arguments, parsed.data(), parsed.size(), count) ||
					count != 2)
					return false;
				uint8_t base = 0;
				uint8_t offset = 0;
				if (!MaterializeAtom(
						parsed[0],
						program,
						variables,
						SCRATCH_REGISTER_A,
						base) ||
					!MaterializeAtom(
						parsed[1],
						program,
						variables,
						SCRATCH_REGISTER_B,
						offset))
					return false;
				program.used_capabilities |= CAPABILITY_MEMORY_READ;
				return AppendInstruction(
					program, memory_opcode, destination, base, offset);
			}

			SdkReadApi api{};
			size_t expected_arguments = 0;
			if (ParseSdkCall(
					expression, api, arguments, expected_arguments))
			{
				if (!SplitArguments(arguments, parsed.data(), parsed.size(), count) ||
					count != expected_arguments)
					return false;
				uint8_t object = 0;
				uint8_t argument = SCRATCH_REGISTER_B;
				if (!MaterializeAtom(
						parsed[0],
						program,
						variables,
						SCRATCH_REGISTER_A,
						object))
					return false;
				if (count == 2)
				{
					if (!MaterializeAtom(
							parsed[1],
							program,
							variables,
							SCRATCH_REGISTER_B,
							argument))
						return false;
				}
				else if (!AppendInstruction(
					program,
					OpCode::LoadImmediate,
					argument,
					0,
					0,
					0,
					0))
					return false;
				program.used_capabilities |= CAPABILITY_SDK_READ;
				return AppendInstruction(
					program,
					OpCode::InvokeSdkRead,
					destination,
					object,
					argument,
					0,
					static_cast<uint64_t>(api));
			}
			return false;
		}

		bool CompileExpression(
			TextView expression,
			ModProgram& program,
			const VariableSet& variables,
			uint8_t destination)
		{
			expression = Trim(expression);
			if (StartsWith(expression, "not "))
			{
				const TextView operand = Trim({
					expression.data + 4,
					expression.size - 4,
				});
				return CompileExpression(
						operand, program, variables, destination) &&
					AppendInstruction(
						program, OpCode::LogicalNot, destination, destination);
			}
			if (expression.size > 1 && expression.data[0] == '-')
			{
				const TextView operand = Trim({
					expression.data + 1,
					expression.size - 1,
				});
				return CompileAtom(
						operand, program, variables, destination) &&
					AppendInstruction(
						program, OpCode::Negate, destination, destination);
			}
			TextView left{};
			TextView right{};
			OpCode binary{};
			if (FindBinaryOperator(expression, left, right, binary))
			{
				uint8_t left_register = 0;
				uint8_t right_register = 0;
				return MaterializeAtom(
						left,
						program,
						variables,
						SCRATCH_REGISTER_A,
						left_register) &&
					MaterializeAtom(
						right,
						program,
						variables,
						SCRATCH_REGISTER_B,
						right_register) &&
					AppendInstruction(
						program,
						binary,
						destination,
						left_register,
						right_register);
			}
			if (CompileCallExpression(
					expression, program, variables, destination))
				return true;
			return CompileAtom(expression, program, variables, destination);
		}

		bool CompileProgramStatement(
			TextView line,
			ModProgram& program,
			VariableSet& variables)
		{
			TextView target{};
			TextView expression{};
			if (ParseAssignment(line, target, expression))
			{
				TextView state_name{};
				if (ParseStateName(target, state_name))
				{
					uint8_t state_index = 0;
					return FindState(program, state_name, state_index) &&
						CompileExpression(
							expression,
							program,
							variables,
							RESULT_REGISTER) &&
						AppendInstruction(
							program,
							OpCode::StoreState,
							RESULT_REGISTER,
							0,
							0,
							0,
							state_index);
				}
				uint8_t destination = 0;
				return AssignVariable(variables, target, destination) &&
					CompileExpression(
						expression, program, variables, destination);
			}

			TextView arguments{};
			std::array<TextView, 4> parsed{};
			size_t count = 0;
			if (ParseCall(line, "equipment.prepare", arguments))
			{
				uint8_t object = 0;
				if (!SplitArguments(arguments, parsed.data(), parsed.size(), count) ||
					count != 1 ||
					!MaterializeAtom(
						parsed[0],
						program,
						variables,
						SCRATCH_REGISTER_A,
						object))
					return false;
				program.used_capabilities |= CAPABILITY_EQUIPMENT;
				return AppendInstruction(
					program, OpCode::PrepareEquipment, object);
			}
			if (ParseCall(line, "combat_clock.forward", arguments))
			{
				if (!SplitArguments(arguments, parsed.data(), parsed.size(), count) ||
					count != 2)
					return false;
				uint8_t pause_type_mask = 0;
				uint8_t state_flags = 0;
				if (!MaterializeAtom(
						parsed[0],
						program,
						variables,
						SCRATCH_REGISTER_A,
						pause_type_mask) ||
					!MaterializeAtom(
						parsed[1],
						program,
						variables,
						SCRATCH_REGISTER_B,
						state_flags))
					return false;
				program.used_capabilities |= CAPABILITY_COMBAT_CLOCK;
				return AppendInstruction(
					program,
					OpCode::ForwardCombatClock,
					pause_type_mask,
					state_flags);
			}
			if (ParseCall(line, "ipc.bind", arguments))
			{
				if (!SplitArguments(arguments, parsed.data(), parsed.size(), count) ||
					count != 2)
					return false;
				uint8_t player_state = NULL_REGISTER;
				uint8_t player_controller = NULL_REGISTER;
				if (!Equals(parsed[0], "None") &&
					!MaterializeAtom(
						parsed[0],
						program,
						variables,
						SCRATCH_REGISTER_A,
						player_state))
					return false;
				if (!Equals(parsed[1], "None") &&
					!MaterializeAtom(
						parsed[1],
						program,
						variables,
						SCRATCH_REGISTER_B,
						player_controller))
					return false;
				if (player_state == NULL_REGISTER &&
					player_controller == NULL_REGISTER)
					return false;
				program.used_capabilities |= CAPABILITY_IPC;
				return AppendInstruction(
					program,
					OpCode::BindIpcContext,
					player_state,
					player_controller);
			}
			if (ParseCall(line, "ipc.emit", arguments))
			{
				if (!SplitArguments(arguments, parsed.data(), parsed.size(), count) ||
					count == 0 || count > 4)
					return false;
				TextView event_name{};
				uint8_t string_index = 0;
				if (!ParseStringLiteral(parsed[0], event_name) ||
					!IsValidEventName(event_name) ||
					!AddStringConstant(program, event_name, string_index))
					return false;
				std::array<uint8_t, NTE_MOD_EVENT_VALUE_COUNT> values{
					NULL_REGISTER,
					NULL_REGISTER,
					NULL_REGISTER,
				};
				constexpr std::array<uint8_t, NTE_MOD_EVENT_VALUE_COUNT> scratch{
					SCRATCH_REGISTER_A,
					SCRATCH_REGISTER_B,
					RESULT_REGISTER,
				};
				for (size_t index = 1; index < count; ++index)
				{
					if (!MaterializeAtom(
							parsed[index],
							program,
							variables,
							scratch[index - 1],
							values[index - 1]))
						return false;
				}
				program.used_capabilities |= CAPABILITY_IPC;
				return AppendInstruction(
					program,
					OpCode::EmitIpcEvent,
					values[0],
					values[1],
					values[2],
					static_cast<uint8_t>(count - 1),
					string_index);
			}
			if (ParseCall(line, "log.info", arguments))
			{
				if (!SplitArguments(arguments, parsed.data(), parsed.size(), count) ||
					count != 1)
					return false;
				TextView message{};
				uint8_t string_index = 0;
				if (!ParseStringLiteral(parsed[0], message) ||
					!AddStringConstant(program, message, string_index))
					return false;
				program.used_capabilities |= CAPABILITY_LOG;
				return AppendInstruction(
					program,
					OpCode::DebugLog,
					0,
					0,
					0,
					0,
					string_index);
			}
			return false;
		}

		bool ParseCondition(
			TextView line,
			const char* prefix,
			TextView& expression)
		{
			size_t prefix_length = 0;
			while (prefix[prefix_length] != '\0')
				++prefix_length;
			if (!StartsWith(line, prefix) || line.size <= prefix_length + 1 ||
				line.data[line.size - 1] != ':')
				return false;
			expression = Trim({
				line.data + prefix_length,
				line.size - prefix_length - 1,
			});
			return expression.size != 0;
		}

		struct ConditionalBlock
		{
			enum class Kind : uint8_t
			{
				Conditional,
				Loop,
			};

			Kind kind;
			size_t false_jump;
			std::array<size_t, 8> end_jumps;
			size_t end_jump_count;
			size_t indentation;
			uint8_t loop_variable;
			size_t loop_check;
		};

		bool PatchConditionalBlock(ModProgram& program, ConditionalBlock& block)
		{
			if (block.kind == ConditionalBlock::Kind::Loop)
			{
				if (!AppendInstruction(
						program,
						OpCode::LoopNext,
						block.loop_variable,
						0,
						0,
						0,
						block.loop_check))
					return false;
				program.instructions[block.loop_check].immediate =
					program.instruction_count;
				return true;
			}
			if (block.false_jump != SIZE_MAX)
				program.instructions[block.false_jump].immediate =
					program.instruction_count;
			for (size_t index = 0; index < block.end_jump_count; ++index)
				program.instructions[block.end_jumps[index]].immediate =
					program.instruction_count;
			return true;
		}

		bool ParseForRange(
			TextView line,
			TextView& variable,
			uint8_t& count)
		{
			if (!StartsWith(line, "for ") ||
				line.size < 18 ||
				line.data[line.size - 1] != ':')
				return false;
			TextView header{
				line.data + 4,
				line.size - 5,
			};
			const char separator[] = " in range(";
			size_t separator_index = header.size;
			for (size_t index = 0;
				index + sizeof(separator) - 1 <= header.size;
				++index)
			{
				bool same = true;
				for (size_t character = 0;
					character < sizeof(separator) - 1;
					++character)
				{
					if (header.data[index + character] !=
						separator[character])
					{
						same = false;
						break;
					}
				}
				if (same)
				{
					separator_index = index;
					break;
				}
			}
			if (separator_index == header.size ||
				header.data[header.size - 1] != ')')
				return false;
			variable = Trim({ header.data, separator_index });
			TextView count_text = Trim({
				header.data + separator_index + sizeof(separator) - 1,
				header.size - separator_index - sizeof(separator),
			});
			uint64_t parsed_count = 0;
			if (!IsValidVariableName(variable) ||
				!ParseInteger(count_text, parsed_count) ||
				parsed_count > 64)
				return false;
			count = static_cast<uint8_t>(parsed_count);
			return true;
		}

		bool CompileConditionalBranch(
			TextView expression,
			ModProgram& program,
			const VariableSet& variables,
			ConditionalBlock& block)
		{
			if (!CompileExpression(
					expression,
					program,
					variables,
					CONDITION_REGISTER))
				return false;
			block.false_jump = program.instruction_count;
			return AppendInstruction(
				program, OpCode::JumpIfFalse, CONDITION_REGISTER);
		}

		bool ParseModProgram(
			TextView text,
			const EnabledMod& expected_mod,
			ModProgram& program)
		{
			TextView line{};
			TextView arguments{};
			TextView value{};
			size_t indentation = 0;
			if (!NextIndentedContentLine(text, line, indentation) ||
				indentation != 0 ||
				!ParseCall(line, "nte_mod", arguments) ||
				!Equals(arguments, "4"))
				return false;
			if (!NextIndentedContentLine(text, line, indentation) ||
				indentation != 0 ||
				!ParseCall(line, "mod", arguments) ||
				!ParseStringLiteral(arguments, value) ||
				!CopyModId(value, program.mod) ||
				!SameModId(program.mod, expected_mod))
				return false;

			bool found_event = false;
			while (NextIndentedContentLine(text, line, indentation))
			{
				if (indentation != 0)
					return false;
				if (ParseCall(line, "requires", arguments) ||
					ParseCall(line, "capability", arguments))
				{
					if (!ParseStringLiteral(arguments, value))
						return false;
					const uint32_t capability = CapabilityFromName(value);
					if (capability == 0 ||
						(program.capabilities & capability) != 0)
						return false;
					program.capabilities |= capability;
					continue;
				}
				TextView target{};
				TextView expression{};
				TextView state_name{};
				uint64_t initial_value = 0;
				if (ParseAssignment(line, target, expression) &&
					ParseStateName(target, state_name))
				{
					if (!ParseInteger(expression, initial_value) ||
						!AddState(program, state_name, initial_value))
						return false;
					continue;
				}
				if (Equals(line, "def on_viewport_tick(event):"))
				{
					found_event = true;
					program.used_capabilities |= CAPABILITY_VIEWPORT_TICK;
					break;
				}
				return false;
			}
			if (!found_event)
				return false;

			VariableSet variables{};
			std::array<ConditionalBlock, 8> blocks{};
			size_t block_count = 0;
			bool has_body = false;
			while (NextIndentedContentLine(text, line, indentation))
			{
				TextView condition{};
				const bool is_else = Equals(line, "else:");
				const bool is_elif = ParseCondition(line, "elif ", condition);
				while (block_count != 0 &&
					indentation <= blocks[block_count - 1].indentation &&
					!(indentation == blocks[block_count - 1].indentation &&
						(is_else || is_elif) &&
						blocks[block_count - 1].kind ==
							ConditionalBlock::Kind::Conditional))
				{
					if (!PatchConditionalBlock(
							program, blocks[--block_count]))
						return false;
				}
				const size_t expected_indentation = block_count == 0
					? 4
					: blocks[block_count - 1].indentation + 4;
				if (is_else || is_elif)
				{
					if (block_count == 0 ||
						indentation != blocks[block_count - 1].indentation ||
						blocks[block_count - 1].kind !=
							ConditionalBlock::Kind::Conditional)
						return false;
					ConditionalBlock& block = blocks[block_count - 1];
					if (block.end_jump_count == block.end_jumps.size() ||
						block.false_jump == SIZE_MAX)
						return false;
					block.end_jumps[block.end_jump_count++] =
						program.instruction_count;
					if (!AppendInstruction(program, OpCode::Jump))
						return false;
					program.instructions[block.false_jump].immediate =
						program.instruction_count;
					block.false_jump = SIZE_MAX;
					if (is_elif &&
						!CompileConditionalBranch(
							condition, program, variables, block))
						return false;
					has_body = true;
					continue;
				}
				if (indentation != expected_indentation)
					return false;
				if (ParseCondition(line, "if ", condition))
				{
					if (block_count == blocks.size())
						return false;
					ConditionalBlock& block = blocks[block_count++];
					block = {};
					block.kind = ConditionalBlock::Kind::Conditional;
					block.false_jump = SIZE_MAX;
					block.indentation = indentation;
					if (!CompileConditionalBranch(
							condition, program, variables, block))
						return false;
					has_body = true;
					continue;
				}
				TextView loop_variable_name{};
				uint8_t loop_count = 0;
				if (ParseForRange(line, loop_variable_name, loop_count))
				{
					if (block_count == blocks.size())
						return false;
					uint8_t loop_variable = 0;
					if (!AssignVariable(
							variables,
							loop_variable_name,
							loop_variable) ||
						!AppendInstruction(
							program,
							OpCode::LoadImmediate,
							loop_variable))
						return false;
					ConditionalBlock& block = blocks[block_count++];
					block = {};
					block.kind = ConditionalBlock::Kind::Loop;
					block.indentation = indentation;
					block.loop_variable = loop_variable;
					block.loop_check = program.instruction_count;
					if (!AppendInstruction(
							program,
							OpCode::JumpIfLoopDone,
							loop_variable,
							loop_count))
						return false;
					has_body = true;
					continue;
				}
				if (!CompileProgramStatement(line, program, variables))
					return false;
				has_body = true;
			}
			while (block_count != 0)
			{
				if (!PatchConditionalBlock(
						program, blocks[--block_count]))
					return false;
			}
			return has_body &&
				program.capabilities == program.used_capabilities;
		}

		bool AppendWide(
			wchar_t* path,
			size_t capacity,
			size_t& length,
			const wchar_t* suffix)
		{
			for (size_t index = 0; suffix[index] != L'\0'; ++index)
			{
				if (length + 1 >= capacity)
					return false;
				path[length++] = suffix[index];
			}
			path[length] = L'\0';
			return true;
		}

		bool CopyWorkspacePath(
			const wchar_t* workspace,
			wchar_t* path,
			size_t capacity,
			size_t& length)
		{
			if (workspace == nullptr || workspace[0] == L'\0')
				return false;
			length = 0;
			while (workspace[length] != L'\0')
			{
				if (length + 1 >= capacity)
					return false;
				path[length] = workspace[length];
				++length;
			}
			if (length == 0 ||
				(path[length - 1] != L'\\' && path[length - 1] != L'/'))
			{
				if (length + 1 >= capacity)
					return false;
				path[length++] = L'\\';
			}
			path[length] = L'\0';
			return true;
		}

		bool BuildConfigPath(
			const wchar_t* workspace,
			wchar_t* path,
			size_t capacity)
		{
			size_t length = 0;
			return CopyWorkspacePath(
					workspace, path, capacity, length) &&
				AppendWide(
					path,
					capacity,
					length,
					NTE_OBFUSCATE_STRING(L"nte-mods.enabled").c_str());
		}

		bool BuildModPath(
			const wchar_t* workspace,
			const EnabledMod& mod,
			wchar_t* path,
			size_t capacity)
		{
			size_t length = 0;
			if (!CopyWorkspacePath(
					workspace, path, capacity, length) ||
				!AppendWide(
					path,
					capacity,
					length,
					NTE_OBFUSCATE_STRING(L"nte-mods\\").c_str()))
				return false;

			for (size_t index = 0; mod.id[index] != '\0'; ++index)
			{
				if (length + 1 >= capacity)
					return false;
				path[length++] = static_cast<wchar_t>(mod.id[index]);
			}
			path[length] = L'\0';
			return AppendWide(
				path,
				capacity,
				length,
				NTE_OBFUSCATE_STRING(L".nte").c_str());
		}

		enum class ReadTextResult
		{
			Ok,
			NotFound,
			Error,
		};

		ReadTextResult ReadTextFile(
			const wchar_t* path,
			std::array<char, MAX_SCRIPT_BYTES + 1>& output,
			size_t& size)
		{
			const HANDLE file = CreateFileW(
				path,
				GENERIC_READ,
				FILE_SHARE_READ,
				nullptr,
				OPEN_EXISTING,
				FILE_ATTRIBUTE_NORMAL,
				nullptr);
			if (file == INVALID_HANDLE_VALUE)
				return GetLastError() == ERROR_FILE_NOT_FOUND
					? ReadTextResult::NotFound
					: ReadTextResult::Error;

			LARGE_INTEGER file_size{};
			const bool valid_size =
				GetFileSizeEx(file, &file_size) &&
				file_size.QuadPart >= 0 &&
				file_size.QuadPart <= MAX_SCRIPT_BYTES;
			if (!valid_size)
			{
				CloseHandle(file);
				return ReadTextResult::Error;
			}

			DWORD bytes_read = 0;
			const DWORD expected = static_cast<DWORD>(file_size.QuadPart);
			const bool read =
				ReadFile(file, output.data(), expected, &bytes_read, nullptr) &&
				bytes_read == expected;
			CloseHandle(file);
			if (!read)
				return ReadTextResult::Error;

			output[bytes_read] = '\0';
			size = bytes_read;
			return ReadTextResult::Ok;
		}

		void* ReadPointerArrayFirst(const void* base, uint64_t offset)
		{
			if (offset > MAX_MEMORY_OFFSET || offset % sizeof(void*) != 0)
				return nullptr;
			PointerArray array{};
			if (!memory::ReadValue(
					base, static_cast<size_t>(offset), array) ||
				array.data == nullptr || array.count < 1 ||
				array.capacity < array.count ||
				!memory::IsReadableRange(array.data, sizeof(*array.data)))
				return nullptr;
			return array.data[0];
		}

		uint64_t ReadPointerArrayCount(const void* base, uint64_t offset)
		{
			if (offset > MAX_MEMORY_OFFSET || offset % sizeof(void*) != 0)
				return 0;
			PointerArray array{};
			if (!memory::ReadValue(
					base, static_cast<size_t>(offset), array) ||
				array.count < 0 || array.capacity < array.count)
				return 0;
			return static_cast<uint64_t>(array.count);
		}

		void ResolveGameSession(TickExecution& execution)
		{
			if (execution.session_resolved)
				return;
			execution.session_resolved = true;
			execution.game_instance = memory::ReadPointer<void>(
				execution.viewport, VIEWPORT_GAME_INSTANCE_OFFSET);
			execution.local_player = ReadPointerArrayFirst(
				execution.game_instance, GAME_INSTANCE_LOCAL_PLAYERS_OFFSET);
			execution.player_controller = memory::ReadPointer<void>(
				execution.local_player, LOCAL_PLAYER_CONTROLLER_OFFSET);
		}

		uint64_t ResolveGameValue(
			TickExecution& execution,
			GameValue value)
		{
			if (value == GameValue::Viewport)
				return reinterpret_cast<uint64_t>(execution.viewport);
			ResolveGameSession(execution);
			switch (value)
			{
			case GameValue::Viewport:
				return reinterpret_cast<uint64_t>(execution.viewport);
			case GameValue::GameInstance:
				return reinterpret_cast<uint64_t>(execution.game_instance);
			case GameValue::LocalPlayer:
				return reinterpret_cast<uint64_t>(execution.local_player);
			case GameValue::PlayerController:
				return reinterpret_cast<uint64_t>(execution.player_controller);
			case GameValue::PlayerState:
				if (!execution.player_state_resolved)
				{
					execution.player_state_resolved = true;
					uint64_t result = 0;
					InvokeSdkReadApi(
						execution.player_controller,
						SdkReadApi::PlayerState,
						0,
						result);
					execution.player_state = reinterpret_cast<void*>(result);
				}
				return reinterpret_cast<uint64_t>(execution.player_state);
			case GameValue::PlayerCharacter:
				if (!execution.player_character_resolved)
				{
					execution.player_character_resolved = true;
					uint64_t result = 0;
					InvokeSdkReadApi(
						execution.player_controller,
						SdkReadApi::PlayerCharacter,
						0,
						result);
					execution.player_character = reinterpret_cast<void*>(result);
				}
				return reinterpret_cast<uint64_t>(execution.player_character);
			}
			return 0;
		}

		uint64_t ResolveCombatClockSample(
			TickExecution& execution,
			void* player_controller)
		{
			if (!execution.combat_clock_sample_resolved ||
				execution.sampled_player_controller != player_controller)
			{
				execution.combat_clock_sample_resolved = true;
				execution.sampled_player_controller = player_controller;
				execution.combat_clock_sample =
					SampleCombatClockState(player_controller);
			}
			return execution.combat_clock_sample;
		}

		template <typename T>
		uint64_t ReadScalar(uint64_t base, uint64_t offset)
		{
			if (offset > MAX_MEMORY_OFFSET)
				return 0;
			T value{};
			if (!memory::ReadValue(
					reinterpret_cast<const void*>(base),
					static_cast<size_t>(offset),
					value))
				return 0;
			return static_cast<uint64_t>(value);
		}

		void MergeIpcPointer(void*& target, uint64_t candidate)
		{
			if (candidate != 0)
				target = reinterpret_cast<void*>(candidate);
		}

		uint64_t CurrentFileTime100ns()
		{
			FILETIME timestamp{};
			GetSystemTimePreciseAsFileTime(&timestamp);
			return (static_cast<uint64_t>(timestamp.dwHighDateTime) << 32) |
				timestamp.dwLowDateTime;
		}

		void RecordModEvent(
			const ModProgram& program,
			const Instruction& instruction,
			const std::array<uint64_t, REGISTER_COUNT>& registers)
		{
			NteModEvent& event = mod_event_history[mod_event_history_next];
			event = {};
			event.sequence = next_mod_event_sequence++;
			event.timestamp_100ns = CurrentFileTime100ns();
			for (size_t index = 0;
				index < NTE_MOD_EVENT_ID_SIZE - 1 &&
				program.mod.id[index] != '\0';
				++index)
				event.mod_id[index] = program.mod.id[index];
			const StringConstant& name =
				program.strings[instruction.immediate];
			for (size_t index = 0; index < name.length; ++index)
				event.name[index] = name.value[index];
			event.value_count = instruction.fourth;
			const std::array<uint8_t, NTE_MOD_EVENT_VALUE_COUNT> value_registers{
				instruction.first,
				instruction.second,
				instruction.third,
			};
			for (uint32_t index = 0; index < event.value_count; ++index)
				event.values[index] = registers[value_registers[index]];
			mod_event_history_next =
				(mod_event_history_next + 1) % NTE_MOD_EVENT_HISTORY_SIZE;
			if (mod_event_history_count < NTE_MOD_EVENT_HISTORY_SIZE)
				++mod_event_history_count;
		}

		void ExecuteProgram(
			ModProgram& program,
			void* viewport,
			TickExecution& execution)
		{
			std::array<uint64_t, REGISTER_COUNT> registers{};
			size_t instruction_index = 0;
			while (instruction_index < program.instruction_count)
			{
				const Instruction& instruction =
					program.instructions[instruction_index];
				auto binary = [&](auto operation) {
					registers[instruction.first] = operation(
						registers[instruction.second],
						registers[instruction.third]);
					++instruction_index;
				};
				switch (instruction.opcode)
				{
				case OpCode::LoadImmediate:
					registers[instruction.first] = instruction.immediate;
					++instruction_index;
					break;
				case OpCode::LoadEventViewport:
					registers[instruction.first] =
						reinterpret_cast<uint64_t>(viewport);
					++instruction_index;
					break;
				case OpCode::LoadGameValue:
					registers[instruction.first] = ResolveGameValue(
						execution,
						static_cast<GameValue>(instruction.immediate));
					++instruction_index;
					break;
				case OpCode::Move:
					registers[instruction.first] = registers[instruction.second];
					++instruction_index;
					break;
				case OpCode::LoadState:
					registers[instruction.first] =
						program.states[instruction.immediate].value;
					++instruction_index;
					break;
				case OpCode::StoreState:
					program.states[instruction.immediate].value =
						registers[instruction.first];
					++instruction_index;
					break;
				case OpCode::Add:
					binary([](uint64_t a, uint64_t b) { return a + b; });
					break;
				case OpCode::Subtract:
					binary([](uint64_t a, uint64_t b) { return a - b; });
					break;
				case OpCode::Multiply:
					binary([](uint64_t a, uint64_t b) { return a * b; });
					break;
				case OpCode::Divide:
					binary([](uint64_t a, uint64_t b) {
						return b == 0 ? 0 : a / b;
					});
					break;
				case OpCode::Modulo:
					binary([](uint64_t a, uint64_t b) {
						return b == 0 ? 0 : a % b;
					});
					break;
				case OpCode::BitAnd:
					binary([](uint64_t a, uint64_t b) { return a & b; });
					break;
				case OpCode::BitOr:
					binary([](uint64_t a, uint64_t b) { return a | b; });
					break;
				case OpCode::BitXor:
					binary([](uint64_t a, uint64_t b) { return a ^ b; });
					break;
				case OpCode::ShiftLeft:
					binary([](uint64_t a, uint64_t b) {
						return b < 64 ? a << b : 0;
					});
					break;
				case OpCode::ShiftRight:
					binary([](uint64_t a, uint64_t b) {
						return b < 64 ? a >> b : 0;
					});
					break;
				case OpCode::Equal:
					binary([](uint64_t a, uint64_t b) { return a == b; });
					break;
				case OpCode::NotEqual:
					binary([](uint64_t a, uint64_t b) { return a != b; });
					break;
				case OpCode::Less:
					binary([](uint64_t a, uint64_t b) {
						return static_cast<int64_t>(a) < static_cast<int64_t>(b);
					});
					break;
				case OpCode::LessEqual:
					binary([](uint64_t a, uint64_t b) {
						return static_cast<int64_t>(a) <= static_cast<int64_t>(b);
					});
					break;
				case OpCode::Greater:
					binary([](uint64_t a, uint64_t b) {
						return static_cast<int64_t>(a) > static_cast<int64_t>(b);
					});
					break;
				case OpCode::GreaterEqual:
					binary([](uint64_t a, uint64_t b) {
						return static_cast<int64_t>(a) >= static_cast<int64_t>(b);
					});
					break;
				case OpCode::LogicalAnd:
					binary([](uint64_t a, uint64_t b) {
						return a != 0 && b != 0;
					});
					break;
				case OpCode::LogicalOr:
					binary([](uint64_t a, uint64_t b) {
						return a != 0 || b != 0;
					});
					break;
				case OpCode::LogicalNot:
					registers[instruction.first] =
						registers[instruction.second] == 0;
					++instruction_index;
					break;
				case OpCode::Negate:
					registers[instruction.first] =
						0 - registers[instruction.second];
					++instruction_index;
					break;
				case OpCode::ReadPointer:
					registers[instruction.first] =
						registers[instruction.third] <= MAX_MEMORY_OFFSET &&
						registers[instruction.third] % sizeof(void*) == 0
						? reinterpret_cast<uint64_t>(memory::ReadPointer<void>(
							reinterpret_cast<const void*>(
								registers[instruction.second]),
							static_cast<size_t>(registers[instruction.third])))
						: 0;
					++instruction_index;
					break;
				case OpCode::ReadU8:
					registers[instruction.first] = ReadScalar<uint8_t>(
						registers[instruction.second],
						registers[instruction.third]);
					++instruction_index;
					break;
				case OpCode::ReadU16:
					registers[instruction.first] = ReadScalar<uint16_t>(
						registers[instruction.second],
						registers[instruction.third]);
					++instruction_index;
					break;
				case OpCode::ReadU32:
					registers[instruction.first] = ReadScalar<uint32_t>(
						registers[instruction.second],
						registers[instruction.third]);
					++instruction_index;
					break;
				case OpCode::ReadU64:
					registers[instruction.first] = ReadScalar<uint64_t>(
						registers[instruction.second],
						registers[instruction.third]);
					++instruction_index;
					break;
				case OpCode::ReadI32:
					registers[instruction.first] = ReadScalar<int32_t>(
						registers[instruction.second],
						registers[instruction.third]);
					++instruction_index;
					break;
				case OpCode::ReadPointerArrayFirst:
					registers[instruction.first] = reinterpret_cast<uint64_t>(
						ReadPointerArrayFirst(
							reinterpret_cast<const void*>(
								registers[instruction.second]),
							registers[instruction.third]));
					++instruction_index;
					break;
				case OpCode::ReadPointerArrayCount:
					registers[instruction.first] = ReadPointerArrayCount(
						reinterpret_cast<const void*>(registers[instruction.second]),
						registers[instruction.third]);
					++instruction_index;
					break;
				case OpCode::IsReadable:
					registers[instruction.first] =
						registers[instruction.third] != 0 &&
						registers[instruction.third] <= MAX_MEMORY_OFFSET &&
						memory::IsReadableRange(
							reinterpret_cast<const void*>(
								registers[instruction.second]),
							static_cast<size_t>(registers[instruction.third]));
					++instruction_index;
					break;
				case OpCode::TickMilliseconds:
					registers[instruction.first] = GetTickCount64();
					++instruction_index;
					break;
				case OpCode::EquipmentCacheMissing:
					registers[instruction.first] = !IsEquipmentRpcCacheReady();
					++instruction_index;
					break;
				case OpCode::EquipmentCacheReady:
				{
					PluginContext context{
						reinterpret_cast<void*>(registers[instruction.second]),
						nullptr,
					};
					registers[instruction.first] =
						IsEquipmentRpcCacheReadyFor(&context);
					++instruction_index;
					break;
				}
				case OpCode::InvokeSdkRead:
				{
					uint64_t result = 0;
					InvokeSdkReadApi(
						reinterpret_cast<void*>(registers[instruction.second]),
						static_cast<SdkReadApi>(instruction.immediate),
						registers[instruction.third],
						result);
					registers[instruction.first] = result;
					++instruction_index;
					break;
				}
				case OpCode::SampleCombatClock:
					registers[instruction.first] = ResolveCombatClockSample(
						execution,
						reinterpret_cast<void*>(registers[instruction.second]));
					++instruction_index;
					break;
				case OpCode::ReadCombatClockPauseMask:
					registers[instruction.first] = static_cast<uint32_t>(
						ResolveCombatClockSample(
							execution,
							reinterpret_cast<void*>(
								registers[instruction.second])));
					++instruction_index;
					break;
				case OpCode::ReadCombatClockStateFlags:
					registers[instruction.first] =
						ResolveCombatClockSample(
							execution,
							reinterpret_cast<void*>(
								registers[instruction.second])) >> 32;
					++instruction_index;
					break;
				case OpCode::JumpIfFalse:
					instruction_index = registers[instruction.first] == 0
						? static_cast<size_t>(instruction.immediate)
						: instruction_index + 1;
					break;
				case OpCode::JumpIfLoopDone:
					instruction_index =
						registers[instruction.first] >= instruction.second
						? static_cast<size_t>(instruction.immediate)
						: instruction_index + 1;
					break;
				case OpCode::Jump:
					instruction_index = static_cast<size_t>(instruction.immediate);
					break;
				case OpCode::LoopNext:
					++registers[instruction.first];
					instruction_index =
						static_cast<size_t>(instruction.immediate);
					break;
				case OpCode::PrepareEquipment:
				{
					PluginContext context{
						reinterpret_cast<void*>(registers[instruction.first]),
						nullptr,
					};
					PrepareEquipmentRpcCache(&context);
					++instruction_index;
					break;
				}
				case OpCode::ForwardCombatClock:
					ForwardCombatClockState(
						static_cast<uint32_t>(registers[instruction.first]),
						static_cast<uint32_t>(registers[instruction.second]));
					++instruction_index;
					break;
				case OpCode::BindIpcContext:
					if (instruction.first != NULL_REGISTER)
						MergeIpcPointer(
							execution.ipc_context.player_state,
							registers[instruction.first]);
					if (instruction.second != NULL_REGISTER)
						MergeIpcPointer(
							execution.ipc_context.player_controller,
							registers[instruction.second]);
					++instruction_index;
					break;
				case OpCode::EmitIpcEvent:
					RecordModEvent(program, instruction, registers);
					++instruction_index;
					break;
				case OpCode::DebugLog:
					DebugLog(program.strings[instruction.immediate].value.data());
					++instruction_index;
					break;
				}
			}
		}
	} // namespace

	ReloadResult ReloadEnabledPrograms(const wchar_t* workspace)
	{
		if (!BuildConfigPath(
			workspace, script_path.data(), script_path.size()))
			return ReloadResult::Error;

		uint64_t fingerprint = 0xCBF29CE484222325ull;
		size_t text_size = 0;
		const ReadTextResult config_result =
			ReadTextFile(script_path.data(), script_text, text_size);
		enabled_mod_set = {};
		if (config_result == ReadTextResult::Ok)
		{
			fingerprint = UpdateFingerprint(
				fingerprint, script_text.data(), text_size);
			if (!ParseEnabledModSet(
				StripUtf8Bom({ script_text.data(), text_size }),
				enabled_mod_set))
			{
				DebugLog(NTE_OBFUSCATE_STRING(
					L"NTE Mods plugin: invalid enabled-mod set.\n").c_str());
				enabled_mod_set = {};
			}
		}
		else if (config_result == ReadTextResult::NotFound)
		{
			constexpr char missing_config[] = "missing-enabled-mod-set";
			fingerprint = UpdateFingerprint(
				fingerprint, missing_config, sizeof(missing_config) - 1);
		}
		else
			return ReloadResult::Error;

		size_t candidate_count = 0;
		uint32_t candidate_capabilities = 0;
		ZeroMemory(candidate_programs.data(), sizeof(candidate_programs));
		for (size_t index = 0; index < enabled_mod_set.count; ++index)
		{
			fingerprint = UpdateFingerprint(
				fingerprint,
				enabled_mod_set.mods[index].id.data(),
				enabled_mod_set.mods[index].id.size());
			if (!BuildModPath(
				workspace,
				enabled_mod_set.mods[index],
				script_path.data(),
				script_path.size()))
				return ReloadResult::Error;

			text_size = 0;
			const ReadTextResult program_result = ReadTextFile(
				script_path.data(), script_text, text_size);
			if (program_result != ReadTextResult::Ok)
			{
				constexpr char missing_program[] = "missing-mod-program";
				fingerprint = UpdateFingerprint(
					fingerprint,
					missing_program,
					sizeof(missing_program) - 1);
				DebugLog(NTE_OBFUSCATE_STRING(
					L"NTE Mods plugin: enabled mod program is unavailable.\n")
					.c_str());
				continue;
			}
			fingerprint = UpdateFingerprint(
				fingerprint, script_text.data(), text_size);

			ZeroMemory(&candidate_program, sizeof(candidate_program));
			if (!ParseModProgram(
				StripUtf8Bom({ script_text.data(), text_size }),
				enabled_mod_set.mods[index],
				candidate_program))
			{
				DebugLog(NTE_OBFUSCATE_STRING(
					L"NTE Mods plugin: invalid mod program.\n").c_str());
				continue;
			}
			candidate_programs[candidate_count++] = candidate_program;
			candidate_capabilities |= candidate_program.capabilities;
		}
		if (fingerprint == source_fingerprint)
			return ReloadResult::Unchanged;

		AcquireSRWLockExclusive(&program_lock);
		programs = candidate_programs;
		program_count = candidate_count;
		enabled_capabilities = candidate_capabilities;
		source_fingerprint = fingerprint;
		ZeroMemory(mod_event_history.data(), sizeof(mod_event_history));
		mod_event_history_count = 0;
		mod_event_history_next = 0;
		next_mod_event_sequence = 1;
		ReleaseSRWLockExclusive(&program_lock);
		return ReloadResult::Changed;
	}

	bool HasViewportTickPrograms()
	{
		AcquireSRWLockShared(&program_lock);
		const bool result = program_count != 0;
		ReleaseSRWLockShared(&program_lock);
		return result;
	}

	uint32_t EnabledCapabilities()
	{
		AcquireSRWLockShared(&program_lock);
		const uint32_t result = enabled_capabilities;
		ReleaseSRWLockShared(&program_lock);
		return result;
	}

	void ExecuteViewportTickPrograms(void* viewport)
	{
		AcquireSRWLockShared(&program_lock);
		TickExecution execution{};
		execution.viewport = viewport;
		for (size_t index = 0; index < program_count; ++index)
			ExecuteProgram(programs[index], viewport, execution);
		if ((enabled_capabilities & CAPABILITY_IPC) != 0)
			PumpLiveIpc(&execution.ipc_context, enabled_capabilities);
		ReleaseSRWLockShared(&program_lock);
	}

	uint32_t CopyModEvents(NteModEvent* output, uint32_t capacity)
	{
		if (output == nullptr || capacity == 0)
			return 0;
		const uint32_t copy_count = capacity < mod_event_history_count
			? capacity
			: mod_event_history_count;
		const uint32_t first =
			(mod_event_history_next +
				NTE_MOD_EVENT_HISTORY_SIZE - copy_count) %
			NTE_MOD_EVENT_HISTORY_SIZE;
		for (uint32_t index = 0; index < copy_count; ++index)
		{
			output[index] = mod_event_history[
				(first + index) % NTE_MOD_EVENT_HISTORY_SIZE];
		}
		return copy_count;
	}

	void Reset()
	{
		AcquireSRWLockExclusive(&program_lock);
		enabled_capabilities = 0;
		program_count = 0;
		source_fingerprint = 0;
		enabled_mod_set = {};
		ZeroMemory(programs.data(), sizeof(programs));
		ZeroMemory(candidate_programs.data(), sizeof(candidate_programs));
		ZeroMemory(mod_event_history.data(), sizeof(mod_event_history));
		mod_event_history_count = 0;
		mod_event_history_next = 0;
		next_mod_event_sequence = 1;
		ReleaseSRWLockExclusive(&program_lock);
	}
} // namespace nte::mods::runtime
