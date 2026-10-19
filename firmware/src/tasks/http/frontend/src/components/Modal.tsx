import { useEffect } from "preact/hooks";
import { ComponentChildren } from "preact";
import { Button } from "./Button";

export function Modal({
	title,
	onClose,
	children,
}: {
	title: string;
	onClose: () => void;
	children: ComponentChildren;
}) {
	useEffect(() => {
		const onKey = (e: KeyboardEvent) => {
			if (e.key === "Escape") onClose();
		};
		window.addEventListener("keydown", onKey);
		return () => window.removeEventListener("keydown", onKey);
	}, [onClose]);

	return (
		<div
			class="fixed inset-0 z-50 flex bg-black/50 backdrop-blur-sm px-4 transition-all"
			onClick={onClose}
		>
			<div
				class="w-full lg:w-2/3 h-full lg:h-2/3 lg:m-auto m-5 flex flex-col rounded-2xl bg-bg border border-text/10 shadow-xl overflow-hidden bg-black/60"
				onClick={(e) => e.stopPropagation()}
			>
				<div class="relative flex items-center px-4 py-3 border-b border-text/10">
					<h2 class="absolute left-1/2 -translate-x-1/2 font-sans text-base font-600 text-text mx-auto">
						{title}
					</h2>
					<div class="ml-auto">
						<Button
							type="button"
							onClick={onClose}
							aria-label="Close"
						>
							Close
						</Button>
					</div>
				</div>
				<div class="flex-1 overflow-y-auto p-4">{children}</div>
			</div>
		</div>
	);
}
