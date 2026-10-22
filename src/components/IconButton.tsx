import type { ButtonHTMLAttributes, ReactNode } from "react";

export default function IconButton({
  label, children, active, className = "", ...props
}: ButtonHTMLAttributes<HTMLButtonElement> & { label: string; children: ReactNode; active?: boolean }) {
  return <button type="button" {...props} className={`icon-button ${active ? "active" : ""} ${className}`}
    title={label} aria-label={label} aria-pressed={active}>{children}</button>;
}
