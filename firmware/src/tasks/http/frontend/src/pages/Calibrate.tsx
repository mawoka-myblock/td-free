import { useEffect, useRef, useState } from "preact/hooks";
import { Pages } from "./types";
import { ButtonLink } from "../components/ButtonLink";
import { Button } from "../components/Button";

type RgbMultipliers = {
	red: number;
	green: number;
	blue: number;
	brightness: number;
	td_reference: number;
	reference_r: number;
	reference_g: number;
	reference_b: number;
};

const DEFAULTS: RgbMultipliers = {
	red: 1.0,
	green: 1.0,
	blue: 1.0,
	brightness: 1.0,
	td_reference: 50.0,
	reference_r: 127,
	reference_g: 127,
	reference_b: 127,
};

function toHex(r: number, g: number, b: number) {
	return `#${r.toString(16).padStart(2, "0")}${g.toString(16).padStart(2, "0")}${b.toString(16).padStart(2, "0")}`.toUpperCase();
}

function hexToRgb(hex: string) {
	const h = hex.replace("#", "");
	return {
		r: parseInt(h.substring(0, 2), 16),
		g: parseInt(h.substring(2, 4), 16),
		b: parseInt(h.substring(4, 6), 16),
	};
}

type SliderRowProps = {
	label: string;
	id: string;
	min: number;
	max: number;
	step: number;
	value: number;
	accentClass: string;
	onChange: (v: number) => void;
};

function SliderRow({
	label,
	min,
	max,
	step,
	value,
	accentClass,
	onChange,
}: SliderRowProps) {
	return (
		<div class="flex items-center gap-3 py-2">
			<span
				class={`font-mono text-sm font-600 w-4 shrink-0 ${accentClass}`}
			>
				{label}
			</span>
			<input
				type="range"
				min={min}
				max={max}
				step={step}
				value={value}
				class="flex-1 accent-current"
				onInput={(e) =>
					onChange(parseFloat((e.target as HTMLInputElement).value))
				}
			/>
			<span class="font-mono text-sm text-text-secondary w-10 text-right tabular-nums">
				{value.toFixed(2)}
			</span>
		</div>
	);
}

type ColorPickerModalProps = {
	initial: { r: number; g: number; b: number };
	onApply: (r: number, g: number, b: number) => void;
	onClose: () => void;
};

function ColorPickerModal({
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

export function CalibrationPage({
	setPage,
}: {
	setPage: (page: Pages) => void;
}) {
	const [mult, setMult] = useState<RgbMultipliers>(DEFAULTS);
	const [calState, setCalState] = useState<
		"idle" | "calibrating" | "success" | "error"
	>("idle");
	const [calMsg, setCalMsg] = useState("");
	const [showPicker, setShowPicker] = useState(false);
	const saveTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

	useEffect(() => {
		fetch("/config/rgb")
			.then((r) => r.json())
			.then((data: Partial<RgbMultipliers>) => {
				setMult((prev) => ({ ...prev, ...data }));
			})
			.catch(() => {});
	}, []);

	function save(next: RgbMultipliers) {
		if (saveTimer.current) clearTimeout(saveTimer.current);
		saveTimer.current = setTimeout(() => {
			fetch("/config/rgb", {
				method: "POST",
				headers: { "Content-Type": "application/json" },
				body: JSON.stringify(next),
			}).catch(() => {});
		}, 300);
	}

	function updateChannel(key: keyof RgbMultipliers, value: number) {
		const next = { ...mult, [key]: value };
		setMult(next);
		save(next);
	}

	function reset() {
		setMult(DEFAULTS);
		save(DEFAULTS);
	}

	async function autoCal() {
		setCalState("calibrating");
		setCalMsg("");
		try {
			const res = await fetch("/config/auto-calibrate", {
				method: "POST",
				headers: { "Content-Type": "application/json" },
				body: JSON.stringify({
					target_r: mult.reference_r,
					target_g: mult.reference_g,
					target_b: mult.reference_b,
				}),
			});
			if (res.status === 428) {
				setCalState("error");
				setCalMsg("No sensor client connected.");
				return;
			}
			if (res.status === 408) {
				setCalState("error");
				setCalMsg("Timed out — insert filament and retry.");
				return;
			}
			const data: Partial<RgbMultipliers> = await res.json();
			const next = { ...mult, ...data };
			setMult(next);
			save(next);
			setCalState("success");
			setTimeout(() => setCalState("idle"), 3000);
		} catch {
			setCalState("error");
			setCalMsg("Request failed.");
		}
	}

	const refHex = toHex(mult.reference_r, mult.reference_g, mult.reference_b);

	const calBtn = {
		idle: { label: "Auto-calibrate", disabled: false },
		calibrating: { label: "Calibrating…", disabled: true },
		success: { label: "Calibrated ✓", disabled: true },
		error: { label: "Retry", disabled: false },
	}[calState];

	return (
		<>
			<header class="flex items-center justify-between h-fit">
				<ButtonLink onClick={() => setPage("dashboard")}>
					← Back
				</ButtonLink>
				<h1 class="font-sans text-2xl font-600 text-text tracking-tight">
					Calibrate
				</h1>
				<div class="w-12" />
			</header>

			{/* Target color */}
			<section class="flex flex-col gap-2">
				<span class="text-xs uppercase tracking-widest text-text-secondary font-500">
					Target color
				</span>
				<div class="flex mx-auto gap-3">
					<button
						class="w-10 h-10 rounded-xl border border-border-secondary shrink-0 transition-transform hover:scale-105"
						style={{ backgroundColor: refHex }}
						onClick={() => setShowPicker(true)}
						aria-label="Pick target color"
					/>
					<span class="font-mono text-sm text-text-secondary my-auto">
						{refHex}
					</span>
					<div class="w-fit">
						<Button onClick={() => setShowPicker(true)}>
							Change
						</Button>
					</div>
				</div>
			</section>

			{/* RGB multipliers */}
			<section class="flex flex-col gap-1 bg-bg-secondary rounded-2xl px-4 py-3">
				<span class="text-xs uppercase tracking-widest text-text-secondary font-500 mb-1">
					Channel multipliers
				</span>
				<SliderRow
					label="R"
					id="red"
					min={0.5}
					max={2.0}
					step={0.01}
					value={mult.red}
					accentClass="text-red-500"
					onChange={(v) => updateChannel("red", v)}
				/>
				<div class="border-t border-border-tertiary" />
				<SliderRow
					label="G"
					id="green"
					min={0.5}
					max={2.0}
					step={0.01}
					value={mult.green}
					accentClass="text-green-600"
					onChange={(v) => updateChannel("green", v)}
				/>
				<div class="border-t border-border-tertiary" />
				<SliderRow
					label="B"
					id="blue"
					min={0.5}
					max={2.0}
					step={0.01}
					value={mult.blue}
					accentClass="text-blue-500"
					onChange={(v) => updateChannel("blue", v)}
				/>
				<div class="border-t border-border-tertiary" />
				<SliderRow
					label="☀"
					id="brightness"
					min={0.1}
					max={3.0}
					step={0.01}
					value={mult.brightness}
					accentClass="text-text-secondary"
					onChange={(v) => updateChannel("brightness", v)}
				/>
			</section>

			{/* Actions */}
			<section class="flex flex-col gap-3">
				<Button
					disabled={calBtn.disabled}
					onClick={
						calState === "error" || calState === "idle"
							? autoCal
							: undefined
					}
				>
					{calBtn.label}
				</Button>

				{calState === "error" && calMsg && (
					<p class="text-xs text-center text-red-600">{calMsg}</p>
				)}

				<Button onClick={reset}>Reset to defaults</Button>
			</section>

			{showPicker && (
				<ColorPickerModal
					initial={{
						r: mult.reference_r,
						g: mult.reference_g,
						b: mult.reference_b,
					}}
					onApply={(r, g, b) => {
						const next = {
							...mult,
							reference_r: r,
							reference_g: g,
							reference_b: b,
						};
						setMult(next);
						save(next);
						setShowPicker(false);
					}}
					onClose={() => setShowPicker(false)}
				/>
			)}
		</>
	);
}
