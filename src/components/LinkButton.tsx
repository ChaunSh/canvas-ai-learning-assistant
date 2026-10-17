import type { ReactNode } from "react";
import { Link, type LinkProps } from "react-router-dom";

import { buttonClass } from "@/components/ui";

/**
 * A router link that looks like a button.
 *
 * Kept separate from `Button` because nesting an anchor inside a `<button>` is
 * invalid HTML and breaks keyboard activation.
 */
export function LinkButton({
  to,
  variant = "secondary",
  size = "md",
  icon,
  children,
  className,
  ...rest
}: LinkProps & {
  variant?: "primary" | "secondary" | "ghost" | "danger";
  size?: "sm" | "md";
  icon?: ReactNode;
  children: ReactNode;
}) {
  return (
    <Link to={to} className={buttonClass(variant, size, className)} {...rest}>
      {icon}
      {children}
    </Link>
  );
}
