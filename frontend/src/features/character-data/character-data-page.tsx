import { createPortal } from "react-dom";
import { useDeferredValue, useMemo, useState } from "react";
import {
  BadgeCheck,
  CircleUserRound,
  Plus,
  RefreshCw,
  Save,
  Search,
  TriangleAlert,
  UserRound,
  X,
} from "lucide-react";

import {
  Alert,
  AlertAction,
  AlertDescription,
  AlertTitle,
} from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty";
import { Skeleton } from "@/components/ui/skeleton";
import { characterAvatarPathUrl } from "@/lib/character-avatar";
import { t, tf } from "@/lib/i18n";
import type {
  CharacterDataRecord,
  CharacterDataRecordInput,
} from "@/lib/tauri/character-data-contract";
import { cn } from "@/lib/utils";

import {
  characterDisplayName,
  characterDraftMatchesRecord,
  characterRecordDraft,
  filterCharacterRecords,
  newCharacterDraft,
} from "./character-data-model";
import {
  type CharacterDataNotice,
  useCharacterData,
} from "./use-character-data";

const INPUT_CLASS =
  "h-9 w-full rounded-lg border bg-background px-3 text-sm outline-none transition-shadow placeholder:text-muted-foreground focus:border-ring focus:ring-3 focus:ring-ring/25 disabled:cursor-not-allowed disabled:bg-muted disabled:text-muted-foreground";
const EMPTY_CHARACTER_RECORDS: CharacterDataRecord[] = [];

export function CharacterDataPage() {
  const model = useCharacterData();
  const [search, setSearch] = useState("");
  const deferredSearch = useDeferredValue(search);
  const [newId, setNewId] = useState("");
  const [selectedId, setSelectedId] = useState<number | null>(null);
  const [draft, setDraft] = useState<CharacterDataRecordInput | null>(null);
  const [localNotice, setLocalNotice] = useState<CharacterDataNotice | null>(
    null,
  );
  const records = model.snapshot?.records ?? EMPTY_CHARACTER_RECORDS;
  const visibleRecords = useMemo(
    () => filterCharacterRecords(records, deferredSearch),
    [deferredSearch, records],
  );
  const selectedRecord =
    selectedId === null
      ? undefined
      : records.find((record) => record.id === selectedId);
  const dirty =
    draft !== null &&
    (draft.originalId === null ||
      !characterDraftMatchesRecord(draft, selectedRecord));
  const notice =
    localNotice ??
    model.notice ??
    (model.snapshot && model.loadError
      ? errorNotice(
          model.loadError.messageKey,
          model.loadError.messageArguments,
          "Character data could not be loaded.",
        )
      : null);

  const warnUnsaved = () => {
    setLocalNotice({
      kind: "error",
      titleKey: "Unsaved changes",
      messageKey: "Save the current changes before switching characters",
      messageArguments: [],
    });
  };

  const selectRecord = (record: CharacterDataRecord) => {
    if (dirty) {
      warnUnsaved();
      return;
    }
    setSelectedId(record.id);
    setDraft(characterRecordDraft(record));
    setLocalNotice(null);
  };

  const addCharacter = () => {
    if (dirty) {
      warnUnsaved();
      return;
    }
    const id = parseCharacterId(newId);
    if (id === null) {
      setLocalNotice(errorNotice("Character ID must be a positive integer."));
      return;
    }
    const existing = records.find((record) => record.id === id);
    if (existing) {
      setSelectedId(existing.id);
      setDraft(characterRecordDraft(existing));
      setLocalNotice(
        errorNotice("Character ID {} already exists.", [String(id)]),
      );
      return;
    }
    setSelectedId(null);
    setDraft(newCharacterDraft(String(id)));
    setNewId("");
    setLocalNotice(null);
  };

  const reload = async () => {
    if (dirty) {
      warnUnsaved();
      return;
    }
    setLocalNotice(null);
    const next = await model.reload();
    if (next && selectedId !== null) {
      const current = next.records.find((record) => record.id === selectedId);
      if (current) setDraft(characterRecordDraft(current));
      else {
        setSelectedId(null);
        setDraft(null);
      }
    }
  };

  const save = async () => {
    if (!draft) return;
    setLocalNotice(null);
    const next = await model.saveRecord(draft);
    if (!next) return;
    const id = Number(draft.id.trim());
    const saved = next.records.find((record) => record.id === id);
    setSelectedId(saved?.id ?? null);
    setDraft(saved ? characterRecordDraft(saved) : null);
  };

  const cancel = () => {
    if (selectedRecord) setDraft(characterRecordDraft(selectedRecord));
    else setDraft(null);
    setLocalNotice(null);
  };

  return (
    <section className="flex size-full min-h-0 min-w-0 flex-col bg-background">
      {notice ? (
        <CharacterDataFloatingNotice
          notice={notice}
          onClose={() => {
            setLocalNotice(null);
            model.clearNotice();
            model.clearLoadError();
          }}
        />
      ) : null}

      <header className="flex shrink-0 items-center justify-between gap-4 border-b px-6 py-4 max-[900px]:px-4">
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <h1 className="text-xl font-semibold tracking-tight">
              {t("Character Data")}
            </h1>
            {model.snapshot ? (
              <Badge variant="outline">
                {tf("Total {} entries", [String(records.length)])}
              </Badge>
            ) : null}
          </div>
          <p className="mt-0.5 text-sm text-muted-foreground">
            {t("Manage character names, attributes, colors and avatars.")}
          </p>
        </div>
        <Button
          disabled={model.loading || model.saving}
          variant="outline"
          onClick={() => void reload()}
        >
          <RefreshCw
            className={cn("size-4", model.loading && "animate-spin")}
            aria-hidden="true"
          />
          {t("Reload")}
        </Button>
      </header>

      <div className="flex shrink-0 flex-wrap items-center gap-2 border-b px-6 py-3 max-[900px]:px-4">
        <label className="text-sm font-medium" htmlFor="character-new-id">
          {t("New ID")}
        </label>
        <input
          className={cn(INPUT_CLASS, "w-32")}
          id="character-new-id"
          inputMode="numeric"
          maxLength={10}
          placeholder={t("e.g. 1080")}
          value={newId}
          onChange={(event) => setNewId(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter") addCharacter();
          }}
        />
        <Button
          disabled={model.loading || newId.trim() === ""}
          onClick={addCharacter}
        >
          <Plus aria-hidden="true" />
          {t("Add")}
        </Button>
        {dirty ? (
          <span className="ml-auto text-xs font-medium text-amber-700 dark:text-amber-300">
            {t("Unsaved changes")}
          </span>
        ) : null}
      </div>

      {model.loading && !model.snapshot ? (
        <CharacterDataLoading />
      ) : model.loadError && !model.snapshot ? (
        <CharacterDataLoadError
          message={tf(
            model.loadError.messageKey,
            model.loadError.messageArguments,
          )}
          onRetry={() => void model.reload(false)}
        />
      ) : (
        <div className="grid min-h-0 flex-1 grid-cols-[minmax(260px,340px)_minmax(0,1fr)] max-[980px]:grid-cols-1 max-[980px]:grid-rows-[220px_minmax(0,1fr)]">
          <aside className="flex min-h-0 flex-col border-r max-[980px]:border-r-0 max-[980px]:border-b">
            <div className="relative shrink-0 border-b p-3">
              <Search
                className="pointer-events-none absolute top-1/2 left-6 size-4 -translate-y-1/2 text-muted-foreground"
                aria-hidden="true"
              />
              <input
                className={cn(INPUT_CLASS, "pl-9")}
                aria-label={t("Search")}
                placeholder={t("ID / name / attribute")}
                value={search}
                onChange={(event) => setSearch(event.target.value)}
              />
            </div>
            <div className="min-h-0 flex-1 overflow-y-auto p-2">
              {visibleRecords.length === 0 ? (
                <Empty className="h-full border-0 p-3">
                  <EmptyHeader>
                    <EmptyMedia variant="icon">
                      <Search aria-hidden="true" />
                    </EmptyMedia>
                    <EmptyTitle>
                      {t("No characters match the current search.")}
                    </EmptyTitle>
                  </EmptyHeader>
                </Empty>
              ) : (
                <div className="flex flex-col gap-1">
                  {visibleRecords.map((record) => (
                    <CharacterRow
                      active={selectedId === record.id}
                      key={record.id}
                      record={record}
                      onClick={() => selectRecord(record)}
                    />
                  ))}
                </div>
              )}
            </div>
          </aside>

          <main className="min-h-0 min-w-0 overflow-y-auto">
            {draft ? (
              <CharacterEditor
                attributes={model.snapshot?.attributes ?? []}
                draft={draft}
                dirty={dirty}
                saving={model.saving}
                onCancel={cancel}
                onChange={setDraft}
                onSave={() => void save()}
              />
            ) : (
              <Empty className="h-full min-h-64 rounded-none border-0">
                <EmptyHeader>
                  <EmptyMedia variant="icon">
                    <UserRound aria-hidden="true" />
                  </EmptyMedia>
                  <EmptyTitle>{t("Select or add a character")}</EmptyTitle>
                  <EmptyDescription>
                    {t(
                      "Select a record on the left, or enter a new ID and click Add.",
                    )}
                  </EmptyDescription>
                </EmptyHeader>
              </Empty>
            )}
          </main>
        </div>
      )}
    </section>
  );
}

function CharacterRow({
  active,
  record,
  onClick,
}: {
  active: boolean;
  record: CharacterDataRecord;
  onClick: () => void;
}) {
  const avatar = characterAvatarPathUrl(record.avatar);
  return (
    <button
      type="button"
      className={cn(
        "flex w-full items-center gap-3 rounded-lg px-3 py-2 text-left transition-colors [content-visibility:auto] [contain-intrinsic-size:56px] hover:bg-muted",
        active && "bg-primary text-primary-foreground hover:bg-primary/90",
      )}
      onClick={onClick}
    >
      <CharacterAvatar avatar={avatar} name={characterDisplayName(record)} />
      <span className="min-w-0 flex-1">
        <span className="flex items-center gap-1.5">
          <span className="truncate text-sm font-medium">
            {characterDisplayName(record)}
          </span>
          {record.verified ? (
            <BadgeCheck className="size-3.5 shrink-0" aria-hidden="true" />
          ) : null}
        </span>
        <span
          className={cn(
            "block truncate text-xs text-muted-foreground",
            active && "text-primary-foreground/70",
          )}
        >
          ID {record.id}
          {record.nameEn ? ` · ${record.nameEn}` : ""}
          {record.attribute ? ` · ${record.attribute}` : ""}
        </span>
      </span>
    </button>
  );
}

function CharacterEditor({
  attributes,
  draft,
  dirty,
  saving,
  onCancel,
  onChange,
  onSave,
}: {
  attributes: string[];
  draft: CharacterDataRecordInput;
  dirty: boolean;
  saving: boolean;
  onCancel: () => void;
  onChange: (draft: CharacterDataRecordInput) => void;
  onSave: () => void;
}) {
  const avatar = characterAvatarPathUrl(draft.avatar);
  const update = <K extends keyof CharacterDataRecordInput>(
    field: K,
    value: CharacterDataRecordInput[K],
  ) => onChange({ ...draft, [field]: value });
  return (
    <form
      className="flex min-h-full flex-col"
      onSubmit={(event) => {
        event.preventDefault();
        onSave();
      }}
    >
      <div className="flex items-center gap-4 border-b px-6 py-5 max-[900px]:px-4">
        <CharacterAvatar
          avatar={avatar}
          large
          name={draft.nameZh || draft.nameEn || draft.id}
        />
        <div className="min-w-0 flex-1">
          <h2 className="truncate text-lg font-semibold">
            {draft.originalId === null
              ? t("Add Character")
              : t("Edit Character")}
          </h2>
          <p className="truncate text-sm text-muted-foreground">
            {draft.nameZh || draft.nameEn || `${t("Character ID")} ${draft.id}`}
          </p>
        </div>
        {draft.attribute ? (
          <Badge variant="outline">{draft.attribute}</Badge>
        ) : null}
        {draft.verified ? <Badge>{t("Verified")}</Badge> : null}
      </div>

      <div className="grid flex-1 content-start grid-cols-2 gap-x-5 gap-y-4 px-6 py-5 max-[900px]:grid-cols-1 max-[900px]:px-4">
        <CharacterField label={t("Character ID")}>
          <input
            className={INPUT_CLASS}
            disabled={draft.originalId !== null}
            inputMode="numeric"
            maxLength={10}
            value={draft.id}
            onChange={(event) => update("id", event.target.value)}
          />
        </CharacterField>
        <CharacterField label={t("Attribute")}>
          <select
            className={INPUT_CLASS}
            value={draft.attribute}
            onChange={(event) => update("attribute", event.target.value)}
          >
            <option value="">{t("Not set")}</option>
            {attributes.map((attribute) => (
              <option key={attribute} value={attribute}>
                {attribute}
              </option>
            ))}
          </select>
        </CharacterField>
        <CharacterField label={t("Chinese Name")}>
          <input
            className={INPUT_CLASS}
            maxLength={128}
            value={draft.nameZh}
            onChange={(event) => update("nameZh", event.target.value)}
          />
        </CharacterField>
        <CharacterField label={t("English Name")}>
          <input
            className={INPUT_CLASS}
            maxLength={128}
            value={draft.nameEn}
            onChange={(event) => update("nameEn", event.target.value)}
          />
        </CharacterField>
        <CharacterField label={t("Codename")}>
          <input
            className={INPUT_CLASS}
            maxLength={128}
            value={draft.codename}
            onChange={(event) => update("codename", event.target.value)}
          />
        </CharacterField>
        <CharacterField label={t("Color")}>
          <div className="flex gap-2">
            <span
              className="size-9 shrink-0 rounded-lg border"
              style={{
                backgroundColor: validColor(draft.color)
                  ? draft.color
                  : "transparent",
              }}
              aria-hidden="true"
            />
            <input
              className={INPUT_CLASS}
              maxLength={7}
              placeholder="#RRGGBB"
              value={draft.color}
              onChange={(event) => update("color", event.target.value)}
            />
          </div>
        </CharacterField>
        <CharacterField
          className="col-span-2 max-[900px]:col-span-1"
          label={t("Avatar Path")}
        >
          <input
            className={INPUT_CLASS}
            maxLength={512}
            placeholder="res/images/characters/player_000.png"
            value={draft.avatar}
            onChange={(event) => update("avatar", event.target.value)}
          />
        </CharacterField>
        <label className="col-span-2 flex h-10 items-center gap-3 rounded-lg border px-3 text-sm max-[900px]:col-span-1">
          <input
            className="size-4 accent-primary"
            checked={draft.verified}
            type="checkbox"
            onChange={(event) => update("verified", event.target.checked)}
          />
          <span>{t("Verified")}</span>
        </label>
      </div>

      <footer className="sticky bottom-0 flex shrink-0 items-center justify-between gap-3 border-t bg-background/95 px-6 py-3 backdrop-blur max-[900px]:px-4">
        <span className="text-xs text-muted-foreground">
          {dirty ? t("Unsaved changes") : t("characters.json saved")}
        </span>
        <div className="flex gap-2">
          <Button
            disabled={!dirty || saving}
            type="button"
            variant="outline"
            onClick={onCancel}
          >
            {t("Cancel Changes")}
          </Button>
          <Button disabled={!dirty || saving} type="submit">
            <Save aria-hidden="true" />
            {t("Save to characters.json")}
          </Button>
        </div>
      </footer>
    </form>
  );
}

function CharacterField({
  children,
  className,
  label,
}: {
  children: React.ReactNode;
  className?: string;
  label: string;
}) {
  return (
    <label className={cn("flex min-w-0 flex-col gap-1.5", className)}>
      <span className="text-sm font-medium">{label}</span>
      {children}
    </label>
  );
}

function CharacterAvatar({
  avatar,
  large = false,
  name,
}: {
  avatar: string | null;
  large?: boolean;
  name: string;
}) {
  return avatar ? (
    <img
      alt=""
      className={cn(
        "size-10 shrink-0 rounded-full border bg-muted object-cover",
        large && "size-14",
      )}
      draggable={false}
      src={avatar}
    />
  ) : (
    <span
      className={cn(
        "flex size-10 shrink-0 items-center justify-center rounded-full border bg-muted text-sm font-semibold",
        large && "size-14 text-lg",
      )}
      aria-label={name}
    >
      {name.trim().slice(0, 1).toLocaleUpperCase() || (
        <CircleUserRound className="size-5" aria-hidden="true" />
      )}
    </span>
  );
}

function CharacterDataLoading() {
  return (
    <div className="grid min-h-0 flex-1 grid-cols-[320px_minmax(0,1fr)] gap-5 p-6">
      <div className="flex flex-col gap-2">
        {Array.from({ length: 7 }, (_, index) => (
          <Skeleton className="h-14 w-full rounded-lg" key={index} />
        ))}
      </div>
      <div className="grid content-start grid-cols-2 gap-4">
        {Array.from({ length: 8 }, (_, index) => (
          <Skeleton className="h-14 w-full rounded-lg" key={index} />
        ))}
      </div>
    </div>
  );
}

function CharacterDataLoadError({
  message,
  onRetry,
}: {
  message: string;
  onRetry: () => void;
}) {
  return (
    <div className="flex min-h-0 flex-1 items-start justify-center p-6">
      <Alert className="max-w-xl" variant="destructive">
        <TriangleAlert aria-hidden="true" />
        <AlertTitle>{t("Character data could not be loaded.")}</AlertTitle>
        <AlertDescription>{message}</AlertDescription>
        <AlertAction>
          <Button size="sm" variant="outline" onClick={onRetry}>
            {t("Retry")}
          </Button>
        </AlertAction>
      </Alert>
    </div>
  );
}

function CharacterDataFloatingNotice({
  notice,
  onClose,
}: {
  notice: CharacterDataNotice;
  onClose: () => void;
}) {
  return createPortal(
    <div className="pointer-events-none fixed inset-x-0 top-4 z-[100] flex justify-center px-4">
      <Alert
        className="pointer-events-auto w-full max-w-lg bg-background pr-10 shadow-xl"
        variant={notice.kind === "error" ? "destructive" : "default"}
      >
        {notice.kind === "error" ? (
          <TriangleAlert aria-hidden="true" />
        ) : (
          <BadgeCheck aria-hidden="true" />
        )}
        <AlertTitle>{t(notice.titleKey)}</AlertTitle>
        <AlertDescription>
          {tf(notice.messageKey, notice.messageArguments)}
        </AlertDescription>
        <AlertAction>
          <Button
            aria-label={t("Dismiss")}
            size="icon-xs"
            variant="ghost"
            onClick={onClose}
          >
            <X aria-hidden="true" />
          </Button>
        </AlertAction>
      </Alert>
    </div>,
    document.body,
  );
}

function parseCharacterId(value: string): number | null {
  if (!/^[1-9]\d*$/.test(value.trim())) return null;
  const parsed = Number(value.trim());
  return Number.isSafeInteger(parsed) && parsed <= 0xffff_ffff ? parsed : null;
}

function validColor(value: string): boolean {
  return /^#[0-9a-fA-F]{6}$/.test(value.trim());
}

function errorNotice(
  messageKey: string,
  messageArguments: string[] = [],
  titleKey = "Character data was not saved",
): CharacterDataNotice {
  return {
    kind: "error",
    titleKey,
    messageKey,
    messageArguments,
  };
}
