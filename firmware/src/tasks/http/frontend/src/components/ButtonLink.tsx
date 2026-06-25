import { ComponentChildren } from "preact";

export function ButtonLink({
	onClick,
	children,
}: {
	onClick?: (event: MouseEvent) => void;
	children?: ComponentChildren;
}) {
	return (
		<button
			class="bg-transparent border-none text-blue-500 p-0 text-base hover:cursor-pointer"
			onClick={onClick}
		>
			{children}
		</button>
	);
}
