import { useId, type ReactNode } from "react";

export function FormRow({
  label,
  description,
  children,
}: {
  label: ReactNode;
  description?: ReactNode;
  children: ReactNode;
}) {
  const id = useId();
  return (
    <div
      className="ui-form-row"
      role="group"
      aria-labelledby={`${id}-label`}
      aria-describedby={description ? `${id}-description` : undefined}
    >
      <div className="ui-form-row-layout">
        <div>
          <div className="ui-form-label" id={`${id}-label`}>
            {label}
          </div>
          {description ? (
            <p className="ui-form-description" id={`${id}-description`}>
              {description}
            </p>
          ) : null}
        </div>
        <div className="ui-form-control">{children}</div>
      </div>
    </div>
  );
}
