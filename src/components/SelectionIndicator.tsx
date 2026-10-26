import { useLayoutEffect, useRef } from "react";
import { SELECTION_MOTION, selectionTransform } from "../lib/motion";
import type { MotionRect } from "../lib/motion";

const targets = {
  tabs: ".document-tab.selected",
  modes: 'button[aria-current="page"]',
  tools: 'button[aria-pressed="true"]',
};

export default function SelectionIndicator({ kind, selectionKey }: {
  kind: keyof typeof targets;
  selectionKey: string;
}) {
  const ref = useRef<HTMLSpanElement>(null);
  const previous = useRef<MotionRect | null>(null);
  const layout = useRef<MotionRect | null>(null);

  useLayoutEffect(() => {
    const marker = ref.current;
    const container = marker?.parentElement;
    if (!marker || !container) return;
    const media = matchMedia("(prefers-reduced-motion: reduce)");
    let animation: Animation | null = null;

    function place(travel: boolean) {
      if (!marker || !container) return;
      const target = container.querySelector<HTMLElement>(targets[kind]);
      const bounds = target?.getBoundingClientRect();
      if (!bounds?.width || !bounds.height) {
        animation?.cancel();
        animation = null;
        marker.hidden = true;
        delete container.dataset.motionReady;
        previous.current = null;
        layout.current = null;
        return;
      }
      const parent = container.getBoundingClientRect();
      const border = getComputedStyle(container);
      const next = {
        left: bounds.left - parent.left + container.scrollLeft - Number.parseFloat(border.borderLeftWidth),
        top: bounds.top - parent.top + container.scrollTop - Number.parseFloat(border.borderTopWidth),
        width: bounds.width,
        height: bounds.height,
      };
      container.dataset.motionReady = "true";
      if (!travel && layout.current
        && Math.abs(layout.current.left - next.left) < 0.01
        && Math.abs(layout.current.top - next.top) < 0.01
        && Math.abs(layout.current.width - next.width) < 0.01
        && Math.abs(layout.current.height - next.height) < 0.01) return;
      const from = animation ? marker.getBoundingClientRect() : previous.current;
      animation?.cancel();
      animation = null;
      marker.hidden = false;
      Object.assign(marker.style, {
        left: `${next.left}px`, top: `${next.top}px`,
        width: `${next.width}px`, height: `${next.height}px`,
      });
      layout.current = next;
      const final = marker.getBoundingClientRect();
      const transform = from && selectionTransform(from, final);
      previous.current = final;
      if (travel && transform && !media.matches && document.visibilityState === "visible"
        && typeof marker.animate === "function") {
        const current = marker.animate([
          { transform, transformOrigin: "0 0" },
          { transform: "none", transformOrigin: "0 0" },
        ], SELECTION_MOTION);
        animation = current;
        current.addEventListener("finish", () => {
          if (animation === current) animation = null;
        }, { once: true });
      }
    }

    function settle() {
      animation?.cancel();
      animation = null;
      if (marker && !marker.hidden) previous.current = marker.getBoundingClientRect();
      place(false);
    }

    place(true);
    const observer = new ResizeObserver(() => place(false));
    observer.observe(container);
    const target = container.querySelector<HTMLElement>(targets[kind]);
    if (target) observer.observe(target);
    media.addEventListener("change", settle);
    document.addEventListener("visibilitychange", settle);
    return () => {
      if (!marker.hidden) {
        const bounds = marker.getBoundingClientRect();
        previous.current = bounds.width && bounds.height ? bounds : null;
      }
      animation?.cancel();
      observer.disconnect();
      media.removeEventListener("change", settle);
      document.removeEventListener("visibilitychange", settle);
      delete container.dataset.motionReady;
    };
  }, [kind, selectionKey]);

  return <span ref={ref} className={`selection-indicator selection-${kind}`} aria-hidden="true" hidden />;
}
