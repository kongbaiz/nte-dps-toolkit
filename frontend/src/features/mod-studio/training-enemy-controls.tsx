import { useState } from "react";
import { FormRow } from "@/components/nte/form-row";
import { ActionNotice } from "@/components/nte/action-notice";
import { Card, CardHeader, CardTitle, CardContent } from "@/components/ui/card";
import { Input, NativeSelect } from "@/components/ui/form-controls";
import { Button } from "@/components/ui/button";
import { t } from "@/lib/i18n";
import { invokeCommand } from "@/lib/tauri/command-error";
import type { TrainingEnemies } from "@/lib/tauri/generated/TrainingEnemies";
import type { EnemyAction } from "@/lib/tauri/generated/EnemyAction";
import type { EnemyRequest } from "@/lib/tauri/generated/EnemyRequest";

export function TrainingEnemyControls({
  identity,
  writable,
  pending,
  run,
}: {
  identity: string;
  writable: boolean;
  pending: boolean;
  run: (action: () => Promise<unknown>) => Promise<void>;
}) {
  const [snapshot, setSnapshot] = useState<TrainingEnemies | null>(null);
  const [queued, setQueued] = useState(false);
  const submit = (request: EnemyRequest | null) =>
    void run(async () => {
      // Forget the old target on error or after queuing; a refresh never replays a write.
      setSnapshot(null);
      setQueued(false);
      const next = await invokeCommand<TrainingEnemies | null>(
        "training_enemies",
        { expectedIdentity: identity, request },
      );
      setSnapshot(next);
      setQueued(next === null);
    });
  return (
    <Card>
      <CardHeader>
        <CardTitle>{t("Training enemies")}</CardTitle>
      </CardHeader>
      <CardContent className="space-y-3">
        <Button
          disabled={pending}
          variant="outline"
          onClick={() => submit(null)}
        >
          {t("Refresh enemy list")}
        </Button>
        {queued && (
          <ActionNotice
            compact
            status="success"
            message={t(
              "Request queued. Refresh to read the result before issuing another command.",
            )}
          />
        )}
        {snapshot && (
          <>
            {snapshot.message && (
              <p role="status" className="text-sm">
                {snapshot.message}
              </p>
            )}
            {snapshot.error && (
              <ActionNotice compact status="error" message={snapshot.error} />
            )}
            <EnemyEditor
              data={snapshot}
              disabled={pending || !writable}
              send={(action) => submit({ provider: snapshot.provider, action })}
            />
          </>
        )}
      </CardContent>
    </Card>
  );
}

function EnemyEditor({
  data,
  disabled,
  send,
}: {
  data: TrainingEnemies;
  disabled: boolean;
  send: (action: EnemyAction) => void;
}) {
  const [custom, setCustom] = useState(data.templates.length === 0);
  const [targetId, setTargetId] = useState("");
  const [attribute, setAttribute] = useState(0);
  const target = data.targets.find(
    (x) => `${x.index}:${x.serial}` === targetId,
  );
  return (
    <div className="space-y-4">
      <form
        className="space-y-2"
        onSubmit={(e) => {
          e.preventDefault();
          const fields = new FormData(e.currentTarget);
          const presets: [number, number][] = [];
          fields.forEach((value, name) => {
            if (name.startsWith("attr-") && value !== "")
              presets.push([Number(name.slice(5)), Number(value)]);
          });
          send(
            custom
              ? {
                  kind: "custom",
                  blueprint: String(fields.get("blueprint")),
                  level: Number(fields.get("level")),
                  pack: String(fields.get("pack")),
                  presets,
                }
              : {
                  kind: "spawn",
                  template: Number(fields.get("template")),
                  presets,
                },
          );
        }}
      >
        <FormRow label={t("Enemy template")}>
          <NativeSelect
            aria-label={t("Enemy template")}
            name="template"
            disabled={disabled}
            onChange={(e) => setCustom(e.target.value === "custom")}
          >
            {data.templates.map((name, i) => (
              <option key={i} value={i}>
                {name}
              </option>
            ))}
            <option value="custom">{t("Custom enemy")}</option>
          </NativeSelect>
        </FormRow>
        {custom && (
          <>
            <FormRow label={t("Blueprint class path")}>
              <Input
                required
                name="blueprint"
                aria-label={t("Blueprint class path")}
                placeholder="/Game/Blueprints/Character/Monster/…"
              />
            </FormRow>
            <FormRow label={t("Level")}>
              <Input
                required
                type="number"
                name="level"
                aria-label={t("Level")}
                min={1}
                max={1000}
                defaultValue={1}
              />
            </FormRow>
            <FormRow label={t("Attribute pack ID")}>
              <Input required name="pack" aria-label={t("Attribute pack ID")} />
            </FormRow>
          </>
        )}
        <details>
          <summary className="cursor-pointer text-sm">
            {t("Attributes before spawn")}
          </summary>
          {data.attributes.map((name, i) =>
            i === 2 || i >= 4 ? (
              <FormRow key={i} label={name}>
                <Input
                  name={`attr-${i}`}
                  type="number"
                  step="any"
                  aria-label={name}
                />
              </FormRow>
            ) : null,
          )}
        </details>
        <Button
          type="submit"
          disabled={disabled || (!custom && !data.templates.length)}
        >
          {t("Spawn enemy")}
        </Button>
      </form>
      <FormRow label={t("Owned enemies")}>
        <NativeSelect
          aria-label={t("Owned enemies")}
          value={targetId}
          onChange={(e) => setTargetId(e.target.value)}
        >
          <option value="">{t("Select an enemy")}</option>
          {data.targets.map((x) => (
            <option
              value={`${x.index}:${x.serial}`}
              key={`${x.index}:${x.serial}`}
            >
              {x.name} ({x.index}:{x.serial})
            </option>
          ))}
        </NativeSelect>
      </FormRow>
      {target && (
        <form
          key={targetId}
          className="space-y-2"
          onSubmit={(e) => {
            e.preventDefault();
            send({
              kind: "edit",
              index: target.index,
              serial: target.serial,
              attribute,
              value: Number(new FormData(e.currentTarget).get("value")),
            });
          }}
        >
          <FormRow label={t("Attribute")}>
            <NativeSelect
              aria-label={t("Attribute")}
              value={attribute}
              onChange={(e) => setAttribute(Number(e.target.value))}
            >
              {data.attributes.map((name, i) => (
                <option key={i} value={i}>
                  {name}
                </option>
              ))}
            </NativeSelect>
          </FormRow>
          <FormRow label={t("Last observed value")}>
            <output>{target.values[attribute] ?? t("Unknown")}</output>
          </FormRow>
          <FormRow label={t("New value")}>
            <Input
              key={attribute}
              required
              name="value"
              aria-label={t("New value")}
              type="number"
              step="any"
            />
          </FormRow>
          <div className="flex flex-wrap gap-2">
            <Button
              disabled={disabled}
              type="button"
              variant="outline"
              onClick={() =>
                send({
                  kind: "read",
                  index: target.index,
                  serial: target.serial,
                })
              }
            >
              {t("Read attributes")}
            </Button>
            <Button disabled={disabled} type="submit">
              {t("Apply value")}
            </Button>
            <Button
              disabled={disabled}
              type="button"
              variant="outline"
              onClick={() =>
                send({
                  kind: "remove",
                  index: target.index,
                  serial: target.serial,
                })
              }
            >
              {t("Remove selected enemy")}
            </Button>
          </div>
        </form>
      )}
    </div>
  );
}
