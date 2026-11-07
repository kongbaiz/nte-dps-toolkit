"""Export effect display text through explicit game-table references, never class-name guesses."""
import argparse
import hashlib
import json
import math
import re
from pathlib import Path

MAX_SOURCE_TABLE_BYTES = 64 * 1024 * 1024

def text(value):
    if not isinstance(value, dict):
        return ""
    return next((value[k] for k in ("SourceString", "LocalizedString", "CultureInvariantString")
                 if isinstance(value.get(k), str) and value[k].strip()), "")

def effect_key(value):
    if not isinstance(value, dict):
        return None
    path = value.get("AssetPathName", "")
    if path.startswith("/Game/"):
        return path.rsplit(".", 1)[-1].removesuffix("_C")
    name = value.get("ObjectName", "")
    if name.startswith("BlueprintGeneratedClass'"):
        return name.split("'")[1].removesuffix("_C")
    return None

def referenced_effects(objects, package, default, paths):
    pending = [default]
    visited, effects = set(), set()
    while pending:
        value = pending.pop()
        if isinstance(value, list):
            pending.extend(value)
        elif isinstance(value, dict):
            ref = value.get("ObjectPath", "")
            key = effect_key(value)
            if key and paths.get(key.lower()) == ref.split(".")[0]:
                effects.add(key)
            if ref.startswith(package + "."):
                index = ref.removeprefix(package + ".")
                if index.isdecimal() and int(index) < len(objects) and index not in visited:
                    visited.add(index)
                    pending.append(objects[int(index)].get("Properties", {}))
            pending.extend(value.values())
    return effects

def build(content):
    content = content.resolve()
    sources, offers = {}, {}
    def document(relative):
        path = (content / relative).resolve()
        if not path.is_relative_to(content) or path.stat().st_size > MAX_SOURCE_TABLE_BYTES:
            raise ValueError("Invalid source table path or size")
        raw = path.read_bytes()
        sources[relative] = hashlib.sha256(raw).hexdigest()
        return json.loads(raw)
    def rows(relative):
        return document(relative)[0]["Rows"]
    def offer(key, name, description, origin, source, priority, parameters=()):
        clean = re.sub(r"<[^>]*>", "", name).strip()
        if not key or not clean or clean == key or clean.lower() in ("none", "todo", "placeholder") or clean.isdecimal():
            return
        offers.setdefault(key.lower(), []).append((priority, {
            "name": name, "description": description, "origin": origin,
            "source": source, "parameters": list(parameters),
        }))
    curve_tables = {}
    def parameter(value):
        if not isinstance(value, dict) or value.get("RegistryType", {}).get("Name", "None") != "None":
            return None
        scale = value.get("Value")
        if type(scale) not in (int, float) or not math.isfinite(scale):
            return None
        curve = value.get("curve", {})
        table = curve.get("CurveTable")
        if not table:
            return scale
        path = table.get("ObjectPath", "").split(".")[0]
        if not path.startswith("/Game/"):
            return None
        relative = path.removeprefix("/Game/") + ".json"
        if relative not in curve_tables:
            curve_tables[relative] = rows(relative)
        row = curve_tables[relative].get(curve.get("RowName"))
        if not row or row.get("InterpMode") not in ("ERichCurveInterpMode::RCIM_Linear", "ERichCurveInterpMode::RCIM_Constant"):
            return None
        if any(row.get(k) != "ERichCurveExtrapolation::RCCE_Constant" for k in ("PreInfinityExtrap", "PostInfinityExtrap")):
            return None
        points = [[k["Time"], k["Value"]] for k in row["Keys"]]
        if not points or any(not math.isfinite(v) for p in points for v in p) or any(a[0] >= b[0] for a,b in zip(points, points[1:])):
            return None
        # Do not infer a historical magnitude from a current level-dependent curve.
        if len({p[1] for p in points}) != 1:
            return None
        return scale * points[0][1]

    tips_path = "DataTable/Skill/Buffer/GameplayEffectTipsDataTable.json"
    for key, row in rows(tips_path).items():
        params = [] if row.get("IsValueOnlyAndCustomDesc") else [parameter(v) for v in row.get("DescParam", [])]
        offer(key, text(row.get("Name")), text(row.get("Desc")), "buff", tips_path+"#"+key, 0,
              params)

    abilities_path = "DataTable/Skill/DT_GameplayAbilityTipsData.json"
    abilities = rows(abilities_path)
    effects_path = "DataTable/Character/DT_CharacterAbilityEffectConfig.json"
    for key,row in rows(effects_path).items():
        tip = abilities.get(key, {})
        desc = "\n".join(text(d.get("Description")) for d in tip.get("AbilityDescription", [])
                         if d.get("AbilityDesType") == "EAbilityDesType::ADT_DES")
        offer(effect_key(row.get("GameplayEffectToActivate")), text(tip.get("Name")), desc,
              "ability", effects_path+"#"+key, 1)

    for path in sorted((content/"DataTable/Character/Awaken").glob("*.json")):
        relative = path.relative_to(content).as_posix()
        for key,row in rows(relative).items():
            for effect in row.get("AwakenEffectStructList", []):
                for modify in effect.get("ModifyDataList", []):
                    offer(effect_key(modify.get("Buff")), text(effect.get("Title")), text(effect.get("Desc")),
                          "awakening", relative+"#"+key+"/"+effect["EffectID"], 1)

    path = "DataTable/Equipment/DT_EquipmentSuitData.json"
    for key,row in rows(path).items():
        for suit in row.get("SuitStructList", []):
            offer(effect_key(suit.get("SuitBuff")), text(row.get("SuitTitle")), text(suit.get("SuitBuffDescription")),
                  "equipment", path+"#"+key+"/"+str(suit["SuitCondition"]), 1)

    path = "DataTable/Fork/DT_ForkUpgradeStarDataTable.json"
    curves = rows("DataTable/Fork/CT_ForkBuff.json")
    for key,row in rows(path).items():
        # A GE level is not proof of arc refinement. Only invariant values can be filled here.
        params = []
        for entry in row.get("DataList", []):
            curve = curves.get(entry["NameId"], {})
            if curve.get("InterpMode") not in ("ERichCurveInterpMode::RCIM_Linear", "ERichCurveInterpMode::RCIM_Constant") or any(
                curve.get(k) != "ERichCurveExtrapolation::RCCE_Constant" for k in ("PreInfinityExtrap", "PostInfinityExtrap")
            ):
                params.append(None)
                continue
            values = {k["Value"] for k in curve.get("Keys", [])}
            params.append(next(iter(values)) * (100 if entry.get("bIsPercent") else 1)
                          if len(values) == 1 else None)
        for buff in row.get("Buffs", []):
            offer(effect_key(buff.get("BuffObject")), text(row.get("Title")), text(row.get("Description")),
                  "arc", path+"#"+key, 1, params)

    # Follow actual default-object/subobject references, not skill-name prefixes.
    mapping = rows("DataTable/Skill/DT_GameplayEffectMappingData.json")
    paths = {key.lower(): row["GameplayEffectClass"]["AssetPathName"].split(".")[0]
             for key, row in mapping.items()}
    for path in sorted((content/"Blueprints/Abilities/Player").rglob("*.json")):
        if path.stem not in abilities:
            continue
        relative = path.relative_to(content).as_posix()
        objects = document(relative)
        classes = [o for o in objects if o.get("Type") == "BlueprintGeneratedClass"
                   and o.get("Name") == path.stem + "_C"]
        if len(classes) != 1:
            continue
        package = "/Game/" + relative.removesuffix(".json")
        effects = referenced_effects(objects, package, classes[0].get("ClassDefaultObject", {}), paths)
        tip = abilities[path.stem]
        desc = "\n".join(text(d.get("Description")) for d in tip.get("AbilityDescription", [])
                         if d.get("AbilityDesType") == "EAbilityDesType::ADT_DES")
        for key in effects:
            offer(key, text(tip.get("Name")), desc, "ability", relative+"#default-object-references", 2)

    # This technical effect has an explicit CoolDown tag but no player-facing Buff tip.
    path = "Blueprints/Abilities/CoolDown/GE_CoolDownBase.json"
    for obj in document(path):
        props = obj.get("Properties", {})
        if "CoolDown" in props.get("InheritableGameplayEffectTags", {}).get("CombinedTags", []):
            offer("GE_CoolDownBase", "Cooldown effect", "", "tag", path, 1)

    entries, ambiguous = {}, []
    for key, candidates in sorted(offers.items()):
        priority = min(p for p,_ in candidates)
        candidates = [v for p,v in candidates if p == priority]
        unique = {json.dumps({k:v for k,v in c.items() if k != "source"}, sort_keys=True) for c in candidates}
        if len(unique) == 1:
            entries[key] = candidates[0]
        else:
            ambiguous.append(key)
    return {"sources": sources, "entries": entries, "ambiguous": ambiguous}

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--content-root", type=Path, default=Path("NTE_Assets/CN/HT/Content"))
    parser.add_argument("--output", type=Path, default=Path("res/data/skills/effect_texts.json"))
    args = parser.parse_args()
    result = build(args.content_root)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2)+"\n", encoding="utf8")
    assert json.loads(args.output.read_bytes()) == result
    for source,digest in result["sources"].items():
        assert hashlib.sha256((args.content_root/source).read_bytes()).hexdigest() == digest
    print(f"Effect text catalog: {len(result['entries'])} entries; {len(result['ambiguous'])} ambiguous mappings omitted")

if __name__ == "__main__":
    main()
