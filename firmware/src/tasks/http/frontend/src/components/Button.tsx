import type { ComponentChildren } from "preact";

type ButtonProps = {
	disabled?: boolean;
	flex?: boolean;
	href?: string;
	target?: string;
	type?: "button" | "submit" | "reset";
	children?: ComponentChildren;
	onClick?: (event: MouseEvent) => void;
};

/**
 * Svelte → Preact compatible Button component
 */
export function Button({
	disabled = false,
	flex = false,
	href,
	target = "_self",
	type = "button",
	children,
	onClick,
}: ButtonProps) {
	const baseClass =
		"text-black w-full px-4 py-2 leading-5 transition-all duration-200 transform bg-[#B07156] rounded-sm text-center outline-hidden";

	const disabledClass = disabled
		? "opacity-50 cursor-not-allowed pointer-events-none"
		: "hover:cursor-pointer hover:opacity-80";

	const flexClass = flex ? "flex" : "";
	const justifyClass = flex ? "justify-center" : "";

	const className = `${baseClass} ${disabledClass} ${flexClass} ${justifyClass}`;

	const handleClick = (event: MouseEvent) => {
		if (disabled) {
			event.preventDefault();
			event.stopPropagation();
			return;
		}
		onClick?.(event);
	};

	if (href) {
		return (
			<a
				href={href}
				target={target}
				class={className}
				onClick={handleClick}
				aria-disabled={disabled ? "true" : undefined}
			>
				{children}
			</a>
		);
	}

	return (
		<button
			type={type}
			disabled={disabled}
			class={className}
			onClick={handleClick}
		>
			{children}
		</button>
	);
}
