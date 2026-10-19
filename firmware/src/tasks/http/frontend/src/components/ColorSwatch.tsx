import { useState } from "preact/hooks";

type ColorSwatchProps = {
	hex?: string | null;
};

/** Displays a color swatch + hex value. Click to copy. */
export function ColorSwatch({ hex }: ColorSwatchProps) {
	const [copied, setCopied] = useState(false);

	async function copy() {
		if (!hex) return;

		try {
			await navigator.clipboard.writeText(hex);
			setCopied(true);
			setTimeout(() => setCopied(false), 1500);
		} catch {
			alert("Color: " + hex);
		}
	}

	if (!hex) return null;

	return (
		<div
			onClick={copy}
			title="Click to copy"
			class="inline-flex items-center gap-2 cursor-pointer bg-black p-2 rounded-lg shadow-2xl"
		>
			<div
				class="w-13 h-13 rounded-lg flex-shrink-0 transition border-solid border-2 border-gray-400"
				style={{
					background: hex,
				}}
			/>
			<span
				class="font-mono p-1.5 m-auto rounded-lg text-sm border-solid border-2 border-gray-400"
			>
				{copied ? "Copied!" : hex.toUpperCase()}
			</span>
		</div>
	);
}
