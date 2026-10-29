// Draggable splitter between panes. The parent owns the size; this reports drag deltas.
import { useRef } from "react";

export function Splitter({
  direction,
  onResize,
  onResizeEnd,
}: {
  /** "vertical" = a vertical bar between left/right panes (drag changes width) */
  direction: "vertical" | "horizontal";
  onResize: (delta: number) => void;
  onResizeEnd?: () => void;
}) {
  const start = useRef(0);
  return (
    <div
      className={`cm-splitter ${direction}`}
      onMouseDown={(e) => {
        e.preventDefault();
        start.current = direction === "vertical" ? e.clientX : e.clientY;
        const move = (ev: MouseEvent) => {
          const p = direction === "vertical" ? ev.clientX : ev.clientY;
          const d = p - start.current;
          if (d !== 0) {
            start.current = p;
            onResize(d);
          }
        };
        // a shield over the page while dragging: iframes (message bodies) would swallow the mouse moves
        const shield = document.createElement("div");
        shield.style.cssText = `position:fixed;inset:0;z-index:100000;cursor:${direction === "vertical" ? "col-resize" : "row-resize"}`;
        document.body.appendChild(shield);
        const up = () => {
          window.removeEventListener("mousemove", move);
          window.removeEventListener("mouseup", up);
          shield.remove();
          document.body.style.cursor = "";
          onResizeEnd?.();
        };
        document.body.style.cursor = direction === "vertical" ? "col-resize" : "row-resize";
        window.addEventListener("mousemove", move);
        window.addEventListener("mouseup", up);
      }}
    />
  );
}

export function clamp(v: number, min: number, max: number) {
  return Math.max(min, Math.min(max, v));
}
