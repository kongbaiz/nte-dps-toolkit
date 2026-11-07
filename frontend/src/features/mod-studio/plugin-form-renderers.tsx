import { createContext, useContext } from "react";
import {
  JsonForms,
  JsonFormsDispatch,
  useJsonForms,
  withJsonFormsControlProps,
  withJsonFormsLayoutProps,
} from "@jsonforms/react";
import {
  createDefaultValue,
  coreReducer,
  update,
  generateDefaultUISchema,
  rankWith,
  uiTypeIs,
  type ControlProps,
  type JsonSchema,
  type Layout,
  type LayoutProps,
  type UISchemaElement,
} from "@jsonforms/core";
import { FormRow } from "@/components/nte/form-row";
import { Button } from "@/components/ui/button";
import { Input, NativeSelect } from "@/components/ui/form-controls";
import { Switch } from "@/components/ui/switch";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import type { PluginForm } from "@/lib/tauri/generated/PluginForm";
import { t } from "@/lib/i18n";

const Actions = createContext<{
  form: PluginForm;
  capturing: boolean;
  translate: (key: string) => string;
  invoke: (id: string, data?: unknown) => void;
} | null>(null);

function useActions() {
  const context = useContext(Actions);
  if (!context)
    throw new Error("Plugin form renderer requires its action owner");
  return context;
}

const LayoutRenderer = withJsonFormsLayoutProps(
  ({ uischema, schema, path, visible, enabled, renderers }: LayoutProps) => {
    const { translate } = useActions();
    if (!visible || (uischema.options?.dev && !import.meta.env.DEV))
      return null;
    const layout = uischema as Layout & { label?: string };
    const children = (
      <div
        className={
          uischema.type === "HorizontalLayout"
            ? "flex flex-wrap items-center gap-2"
            : uischema.type === "Group"
              ? "space-y-1"
              : "space-y-4"
        }
      >
        {layout.elements.map((element, index) => (
          <JsonFormsDispatch
            key={index}
            schema={schema}
            uischema={element}
            path={path}
            enabled={enabled}
            renderers={renderers}
          />
        ))}
      </div>
    );
    if (uischema.type !== "Group") return children;
    return (
      <Card size="sm" className="gap-0">
        <CardHeader className="flex flex-row items-center justify-between border-b">
          <CardTitle>{translate(layout.label ?? "")}</CardTitle>
          {uischema.options?.dev && <Badge variant="outline">{t("dev")}</Badge>}
        </CardHeader>
        <CardContent className="pt-2">{children}</CardContent>
      </Card>
    );
  },
);

const ButtonRenderer = withJsonFormsLayoutProps(
  ({ uischema, enabled, visible }: LayoutProps) => {
    const { form, capturing, translate, invoke } = useActions();
    const jsonforms = useJsonForms();
    const action = form.actions.find(
      (item) => item.id === uischema.options?.action,
    );
    if (!visible || !action || (action.dev && !import.meta.env.DEV))
      return null;
    const invalid =
      uischema.options?.validate && (jsonforms.core?.errors?.length ?? 0) > 0;
    return (
      <Button
        variant={uischema.options?.primary ? "default" : "outline"}
        disabled={!enabled || invalid || (action.requiresIdle && capturing)}
        onClick={() => invoke(action.id)}
      >
        {translate(action.label)}
      </Button>
    );
  },
);

const ControlRenderer = withJsonFormsControlProps((props: ControlProps) => {
  const {
    data,
    path,
    handleChange,
    schema,
    rootSchema,
    visible,
    enabled,
    label,
    errors,
    renderers,
  } = props;
  const { form, capturing, translate, invoke } = useActions();
  const jsonforms = useJsonForms();
  if (!visible) return null;
  if (schema.type === "object")
    return (
      <JsonFormsDispatch
        schema={schema}
        uischema={generateDefaultUISchema(schema)}
        path={path}
        enabled={enabled}
        renderers={renderers}
      />
    );
  if (schema.type === "array") {
    const items = (data ?? []) as unknown[];
    const itemSchema = schema.items as JsonSchema;
    return (
      <FormRow label={translate(label)}>
        <div className="space-y-3">
          {items.map((_, index) => (
            <div key={index} className="space-y-2 rounded-lg border p-3">
              <JsonFormsDispatch
                schema={itemSchema}
                uischema={generateDefaultUISchema(itemSchema)}
                path={`${path}.${index}`}
                enabled={enabled}
                renderers={renderers}
              />
              <Button
                variant="outline"
                disabled={!enabled}
                onClick={() =>
                  handleChange(
                    path,
                    items.filter((_, current) => current !== index),
                  )
                }
              >
                {t("Remove")}
              </Button>
            </div>
          ))}
          <Button
            variant="outline"
            disabled={!enabled}
            onClick={() =>
              handleChange(path, [
                ...items,
                createDefaultValue(itemSchema, rootSchema),
              ])
            }
          >
            {t("Add")}
          </Button>
          {errors && (
            <p role="alert" className="text-xs text-destructive">
              {errors}
            </p>
          )}
        </div>
      </FormRow>
    );
  }
  const choices =
    schema.oneOf?.map((item) => ({
      value: item.const,
      title: item.title ?? String(item.const),
    })) ?? schema.enum?.map((value) => ({ value, title: String(value) }));
  let input;
  if (choices) {
    const selected = choices.findIndex((item) => item.value === data);
    input = (
      <NativeSelect
        aria-label={translate(label)}
        disabled={!enabled}
        value={selected < 0 ? "" : String(selected)}
        onChange={(event) =>
          handleChange(
            path,
            event.target.value === ""
              ? undefined
              : choices[Number(event.target.value)].value,
          )
        }
      >
        <option value="">{t("Select")}</option>
        {choices.map((item, index) => (
          <option key={index} value={index}>
            {translate(item.title)}
          </option>
        ))}
      </NativeSelect>
    );
  } else if (schema.type === "boolean") {
    input = (
      <Switch
        aria-label={translate(label)}
        disabled={
          !enabled ||
          (capturing &&
            form.actions.some(
              (action) =>
                action.id === props.uischema.options?.action &&
                action.requiresIdle,
            ))
        }
        checked={data === true}
        onCheckedChange={(value) => {
          const action = props.uischema.options?.action;
          if (action) {
            const next = coreReducer(
              jsonforms.core!,
              update(path, () => value),
            ).data;
            invoke(action, next);
          } else handleChange(path, value);
        }}
      />
    );
  } else {
    const numeric = schema.type === "number" || schema.type === "integer";
    input = (
      <Input
        aria-label={translate(label)}
        disabled={!enabled}
        type={numeric ? "number" : "text"}
        step={schema.type === "integer" ? 1 : "any"}
        value={data ?? ""}
        onChange={(event) =>
          handleChange(
            path,
            numeric
              ? event.target.value === ""
                ? undefined
                : event.target.valueAsNumber
              : event.target.value,
          )
        }
      />
    );
  }
  return (
    <FormRow
      inline={schema.type === "boolean"}
      label={translate(label)}
      description={
        schema.description ? translate(schema.description) : undefined
      }
    >
      <div className="space-y-1">
        {input}
        {errors && (
          <p role="alert" className="text-xs text-destructive">
            {errors}
          </p>
        )}
      </div>
    </FormRow>
  );
});

const renderers = [
  ...["VerticalLayout", "HorizontalLayout", "Group"].map((type) => ({
    tester: rankWith(1, uiTypeIs(type)),
    renderer: LayoutRenderer,
  })),
  { tester: rankWith(1, uiTypeIs("Control")), renderer: ControlRenderer },
  { tester: rankWith(1, uiTypeIs("Button")), renderer: ButtonRenderer },
];

export function PluginFormRenderer({
  form,
  data,
  disabled,
  capturing,
  translate,
  invoke,
  onChange,
}: {
  form: PluginForm;
  data: unknown;
  disabled: boolean;
  capturing: boolean;
  translate: (key: string) => string;
  invoke: (id: string, data?: unknown) => void;
  onChange: (data: unknown) => void;
}) {
  return (
    <Actions.Provider value={{ form, capturing, translate, invoke }}>
      <JsonForms
        schema={form.schema as JsonSchema}
        uischema={form.uischema as UISchemaElement}
        data={data}
        renderers={renderers}
        readonly={disabled}
        onChange={({ data }) => onChange(data)}
      />
    </Actions.Provider>
  );
}
