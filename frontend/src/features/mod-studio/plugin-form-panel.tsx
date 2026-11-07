import type { PluginFormActionRequest } from "@/lib/tauri/generated/PluginFormActionRequest";
import { Component, useEffect, useRef, useState, type ReactNode } from "react";
import { ActionNotice } from "@/components/nte/action-notice";
import { t, useFrontendLanguage } from "@/lib/i18n";
import { invokeCommand } from "@/lib/tauri/command-error";
import type { PluginFormSnapshot } from "@/lib/tauri/generated/PluginFormSnapshot";
import type { ReleaseResult } from "@/lib/tauri/generated/ReleaseResult";
import { PluginFormRenderer } from "./plugin-form-renderers";
import { NativeResponse, OperationConfirmation } from "./native-operation";

class FormBoundary extends Component<
  { children: ReactNode },
  { failed: boolean }
> {
  state = { failed: false };
  static getDerivedStateFromError() {
    return { failed: true };
  }
  render() {
    return this.state.failed ? (
      <ActionNotice
        status="error"
        message={t(
          "The plugin interface could not be rendered. Refresh or update the plugin.",
        )}
      />
    ) : (
      this.props.children
    );
  }
}

export function PluginFormPanel({
  snapshot,
  file,
  identity,
  pending,
  capturing,
  run,
}: {
  snapshot: PluginFormSnapshot;
  file: string;
  identity: string;
  pending: boolean;
  capturing: boolean;
  run: (action: () => Promise<unknown>) => Promise<void>;
}) {
  const language = useFrontendLanguage();
  const [data, setData] = useState<unknown>(snapshot.form.data);
  const [result, setResult] = useState<ReleaseResult | null>(null);
  const [notice, setNotice] = useState(false);
  const [confirmation, setConfirmation] = useState<{
    id: string;
    data: unknown;
  } | null>(null);
  const generation = useRef({ value: 0 });
  const writing = useRef(false);
  useEffect(() => {
    const lifetime = generation.current;
    return () => {
      lifetime.value++;
    };
  }, []);
  const translate = (key: string) =>
    snapshot.form.translations[language]?.[key] ?? t(key);
  const execute = (id: string, inputData?: unknown, confirmed = false) => {
    if (writing.current) return;
    const action = snapshot.form.actions.find((item) => item.id === id);
    if (!action) return;
    if (action.confirmation && !confirmed) {
      setConfirmation({ id, data: inputData === undefined ? data : inputData });
      return;
    }
    const ticket = generation.current.value;
    const previous = data;
    const submitted = inputData === undefined ? data : inputData;
    writing.current = true;
    void run(async () => {
      setData(submitted);
      setNotice(false);
      try {
        const next = await invokeCommand<ReleaseResult>(
          "invoke_plugin_form_action",
          {
            request: {
              file,
              expectedIdentity: identity,
              expectedOwner: snapshot.owner,
              action: id,
              data: submitted,
              confirmed,
            } satisfies PluginFormActionRequest,
          },
        );
        if (ticket === generation.current.value) {
          setResult(action.dev && import.meta.env.DEV ? next : null);
          setNotice(inputData === undefined);
        }
      } catch (error) {
        if (ticket === generation.current.value) setData(previous);
        throw error;
      } finally {
        if (ticket === generation.current.value) setConfirmation(null);
      }
    }).finally(() => {
      writing.current = false;
    });
  };
  const confirmAction = snapshot.form.actions.find(
    (item) => item.id === confirmation?.id,
  );
  return (
    <div className="space-y-3">
      <FormBoundary key={snapshot.owner}>
        <PluginFormRenderer
          form={snapshot.form}
          data={data}
          disabled={pending}
          capturing={capturing}
          translate={translate}
          invoke={(id, next) => execute(id, next)}
          onChange={setData}
        />
      </FormBoundary>
      {notice && (
        <ActionNotice
          status="success"
          compact
          message={t("Request submitted.")}
        />
      )}
      {result && <NativeResponse result={result} />}
      <OperationConfirmation
        message={
          confirmAction?.confirmation
            ? translate(confirmAction.confirmation)
            : null
        }
        pending={pending}
        cancel={() => setConfirmation(null)}
        confirm={() => {
          if (confirmation) execute(confirmation.id, confirmation.data, true);
        }}
      />
    </div>
  );
}
