// "Rule description (click an underlined value to edit)" box.
import type { Line, ValueTarget } from "./describe";

export function RuleDescription({ lines, onEdit, empty }: { lines: Line[]; onEdit?: (t: ValueTarget) => void; empty?: string }) {
  return (
    <div className="cm-rules-desc" role="document">
      {!lines.length && empty && <div className="cm-muted">{empty}</div>}
      {lines.map((l, i) => (
        <div key={i} className="cm-rules-desc-line">
          {l.map((s, j) =>
            "text" in s ? (
              <span key={j}>{s.text}</span>
            ) : (
              <a
                key={j}
                className={`cm-rules-val${s.unset ? " unset" : ""}${onEdit ? "" : " static"}`}
                onClick={(e) => {
                  e.preventDefault();
                  onEdit?.(s.target);
                }}
              >
                {s.value}
              </a>
            ),
          )}
        </div>
      ))}
    </div>
  );
}
