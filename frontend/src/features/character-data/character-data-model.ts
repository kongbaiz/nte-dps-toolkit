import type {
  CharacterDataRecord,
  CharacterDataRecordInput,
} from "@/lib/tauri/character-data-contract";

export function characterRecordDraft(
  record: CharacterDataRecord,
): CharacterDataRecordInput {
  return {
    originalId: String(record.id),
    id: String(record.id),
    nameZh: record.nameZh,
    nameEn: record.nameEn,
    codename: record.codename,
    attribute: record.attribute,
    verified: record.verified,
    color: record.color,
    avatar: record.avatar,
  };
}

export function newCharacterDraft(id: string): CharacterDataRecordInput {
  return {
    originalId: null,
    id,
    nameZh: "",
    nameEn: "",
    codename: "",
    attribute: "",
    verified: false,
    color: "",
    avatar: "",
  };
}

export function characterDraftMatchesRecord(
  draft: CharacterDataRecordInput,
  record: CharacterDataRecord | undefined,
): boolean {
  return (
    record !== undefined &&
    draft.originalId === String(record.id) &&
    draft.id === String(record.id) &&
    draft.nameZh === record.nameZh &&
    draft.nameEn === record.nameEn &&
    draft.codename === record.codename &&
    draft.attribute === record.attribute &&
    draft.verified === record.verified &&
    draft.color === record.color &&
    draft.avatar === record.avatar
  );
}

export function filterCharacterRecords(
  records: CharacterDataRecord[],
  search: string,
): CharacterDataRecord[] {
  const normalized = search.trim().toLocaleLowerCase();
  if (normalized === "") return records;
  return records.filter((record) =>
    [record.id, record.nameZh, record.nameEn, record.codename, record.attribute]
      .join(" ")
      .toLocaleLowerCase()
      .includes(normalized),
  );
}

export function characterDisplayName(record: CharacterDataRecord): string {
  return record.nameZh || record.nameEn || record.codename || String(record.id);
}
