import { useState } from "preact/hooks";
import { Button } from "./Button";
import { toHex } from "../helpers";

type ColorPickerModalProps = {
	initial: { r: number; g: number; b: number };
	onApply: (r: number, g: number, b: number) => void;
	onClose: () => void;
};

function hexToRgb(hex: string) {
	const h = hex.replace("#", "");
	return {
		r: parseInt(h.substring(0, 2), 16),
		g: parseInt(h.substring(2, 4), 16),
		b: parseInt(h.substring(4, 6), 16),
	};
}

export function ColorPickerModal({
	initial,
	onApply,
	onClose,
}: ColorPickerModalProps) {
	const [r, setR] = useState(initial.r);
	const [g, setG] = useState(initial.g);
	const [b, setB] = useState(initial.b);
	const hex = toHex(r, g, b);

	function fromNative(h: string) {
		const c = hexToRgb(h);
		setR(c.r);
		setG(c.g);
		setB(c.b);
	}

	return (
		<div
			class="fixed inset-0 z-50 flex items-center justify-center bg-black/80"
			onClick={(e) => e.target === e.currentTarget && onClose()}
		>
			<div class="bg-bg-primary rounded-2xl shadow-xl p-6 w-72 flex flex-col gap-4">
				<h2 class="font-sans text-base font-600 text-text m-0">
					Target color
				</h2>

				<div class="flex items-center gap-3">
					<div
						class="w-10 h-10 rounded-xl border border-border-secondary cursor-pointer shrink-0"
						style={{ backgroundColor: hex }}
						onClick={() =>
							document.getElementById("native-picker")?.click()
						}
					/>
					<input
						type="color"
						id="native-picker"
						class="hidden"
						value={hex.toLowerCase()}
						onInput={(e) =>
							fromNative((e.target as HTMLInputElement).value)
						}
					/>
					<span class="font-mono text-sm text-text-secondary">
						{hex}
					</span>
				</div>

				<div class="flex flex-col gap-1">
					{[
						{
							label: "R",
							val: r,
							set: setR,
							color: "text-red-500",
						},
						{
							label: "G",
							val: g,
							set: setG,
							color: "text-green-600",
						},
						{
							label: "B",
							val: b,
							set: setB,
							color: "text-blue-500",
						},
					].map(({ label, val, set, color }) => (
						<div key={label} class="flex items-center gap-2">
							<span
								class={`font-mono text-sm font-600 w-3 ${color}`}
							>
								{label}
							</span>
							<input
								type="range"
								min={0}
								max={255}
								step={1}
								value={val}
								class="flex-1"
								onInput={(e) =>
									set(
										parseInt(
											(e.target as HTMLInputElement)
												.value,
										),
									)
								}
							/>
							<input
								type="number"
								min={0}
								max={255}
								value={val}
								class="w-12 font-mono text-sm text-right border border-border-secondary rounded-md px-1 py-0.5 bg-bg-secondary"
								onInput={(e) => {
									const n = Math.max(
										0,
										Math.min(
											255,
											parseInt(
												(e.target as HTMLInputElement)
													.value,
											) || 0,
										),
									);
									set(n);
								}}
							/>
						</div>
					))}
				</div>

				<div class="flex gap-2 pt-1">
					<Button onClick={onClose}>Cancel</Button>
					<Button onClick={() => onApply(r, g, b)}>Apply</Button>
				</div>
			</div>
		</div>
	);
}
