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
			style={{
				display: "inline-flex",
				alignItems: "center",
				gap: "10px",
				cursor: "pointer",
				userSelect: "none",
			}}
		>
			<div
				style={{
					width: 52,
					height: 52,
					borderRadius: 10,
					background: hex,
					border: "2px solid #e2e8f0",
					flexShrink: 0,
					transition: "transform 0.1s",
				}}
			/>
			<span
				style={{
					fontFamily: "'JetBrains Mono', monospace",
					fontSize: 14,
					padding: "4px 10px",
					background: "#f8fafc",
					border: "1px solid #e2e8f0",
					borderRadius: 6,
					letterSpacing: "0.05em",
				}}
			>
				{copied ? "Copied!" : hex.toUpperCase()}
			</span>
		</div>
	);
}
