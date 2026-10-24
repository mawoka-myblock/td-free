import { useEffect, useState } from "preact/hooks";
import { Pages } from "./types";
import { ButtonLink } from "../components/ButtonLink";
import { Button } from "../components/Button";
import { toHex } from "../helpers";
import { ColorPickerModal } from "../components/ColorPicker";
import { MeasurementChanged } from "../api";

type Rgb = [number, number, number];

type ColorReference = {
	/** color printed on the spool */
	target: Rgb;
	/** white balance only, without correction */
	measured: Rgb;
	/** what the device shows now, corrected with all references */
	corrected: Rgb;
};

const MAX_REFERENCES = 8;

const hex = (c: Rgb) => toHex(c[0], c[1], c[2]);

/** CIE76 color difference of two sRGB colors */
function deltaE(a: Rgb, b: Rgb) {
	const lab = (c: Rgb) => {
		const lin = c.map((v) => {
			v /= 255;
			return v <= 0.04045 ? v / 12.92 : Math.pow((v + 0.055) / 1.055, 2.4);
		});
		const x = (0.4124 * lin[0] + 0.3576 * lin[1] + 0.1805 * lin[2]) / 0.95047;
		const y = 0.2126 * lin[0] + 0.7152 * lin[1] + 0.0722 * lin[2];
		const z = (0.0193 * lin[0] + 0.1192 * lin[1] + 0.9505 * lin[2]) / 1.08883;
		const f = (t: number) => (t > 0.008856 ? Math.cbrt(t) : 7.787 * t + 16 / 116);
		return [116 * f(y) - 16, 500 * (f(x) - f(y)), 200 * (f(y) - f(z))];
	};
	const p = lab(a);
	const q = lab(b);
	return Math.hypot(p[0] - q[0], p[1] - q[1], p[2] - q[2]);
}

function correctionMode(n: number) {
	if (n === 0) return "No references yet: the color comes from the white balance only.";
	if (n < 3)
		return `The white point is corrected. Add ${3 - n} more for a full color correction.`;
	return "Full color correction from all references.";
}

function Swatch({ color, label }: { color: Rgb; label: string }) {
	return (
		<div class="flex flex-col gap-1 items-start">
			<span class="text-xs text-text-secondary">{label}</span>
			<div
				class="w-12 h-10 rounded-lg border border-border-secondary"
				style={{ backgroundColor: hex(color) }}
			/>
			<span class="font-mono text-xs text-text-secondary">{hex(color)}</span>
		</div>
	);
}

export function CalibrationPage({
	setPage,
}: {
	setPage: (page: Pages) => void;
}) {
	const [references, setReferences] = useState<ColorReference[]>([]);
	// The color has to be entered for every filament, a stale one from the
	// filament before would teach the device the wrong color.
	const [target, setTarget] = useState<Rgb | null>(null);
	const [showPicker, setShowPicker] = useState(false);
	const [busy, setBusy] = useState(false);
	const [message, setMessage] = useState<{ text: string; error: boolean } | null>(
		null,
	);
	const [measurement, setMeasurement] = useState<MeasurementChanged | null>(
		import.meta.env.PROD
			? null
			: {
					td: "1.4",
					hex_color: "FF0000",
					buf_count: 21,
				},
	);

	useEffect(() => {
		const es = new EventSource("/events/data");
		const handler = (event: MessageEvent) => {
			try {
				setMeasurement(JSON.parse(event.data));
			} catch {
				if (event.data === "no_filament") setMeasurement("no_filament");
			}
		};
		es.addEventListener("measurement_changed", handler as EventListener);
		return () => {
			es.removeEventListener(
				"measurement_changed",
				handler as EventListener,
			);
			es.close();
		};
	}, []);

	useEffect(() => {
		fetch("/config/color-references")
			.then((r) => r.json())
			.then((data: ColorReference[]) => setReferences(data))
			.catch(() => {});
	}, []);

	async function postReferences(path: string, body: object) {
		const res = await fetch(path, {
			method: "POST",
			headers: { "Content-Type": "application/json" },
			body: JSON.stringify(body),
		});
		return res;
	}

	async function addReference() {
		if (!target) return;
		setBusy(true);
		setMessage(null);
		try {
			const res = await postReferences("/config/color-references/add", {
				target_r: target[0],
				target_g: target[1],
				target_b: target[2],
			});
			if (res.status === 408) {
				setMessage({
					text: "No filament found. Insert the filament, wait until the sensor color is stable and try again.",
					error: true,
				});
				return;
			}
			if (res.status === 409) {
				setMessage({
					text: `All ${MAX_REFERENCES} reference slots are used. Remove one first.`,
					error: true,
				});
				return;
			}
			if (res.status === 428) {
				setMessage({
					text: "The page lost its connection to the device. Reload the page and try again.",
					error: true,
				});
				return;
			}
			if (!res.ok) throw new Error(String(res.status));
			setReferences(await res.json());
			setTarget(null);
			setMessage({
				text: "Added. Insert the next filament with a known color, or stop here.",
				error: false,
			});
		} catch {
			setMessage({
				text: "The device didn't answer. Check the connection and try again.",
				error: true,
			});
		} finally {
			setBusy(false);
		}
	}

	async function removeReference(index: number) {
		setMessage(null);
		try {
			const res = await postReferences("/config/color-references/remove", {
				index,
			});
			if (!res.ok) throw new Error(String(res.status));
			setReferences(await res.json());
		} catch {
			setMessage({ text: "Couldn't remove the reference. Try again.", error: true });
		}
	}

	async function clearReferences() {
		setMessage(null);
		try {
			const res = await postReferences("/config/color-references/clear", {});
			if (!res.ok) throw new Error(String(res.status));
			setReferences(await res.json());
		} catch {
			setMessage({ text: "Couldn't remove the references. Try again.", error: true });
		}
	}

	const full = references.length >= MAX_REFERENCES;

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

			{/* How it works */}
			<section class="flex flex-col gap-2 bg-bg-secondary rounded-2xl px-4 py-3">
				<span class="text-xs uppercase tracking-widest text-text-secondary font-500">
					Color calibration with known filaments
				</span>
				<ol class="m-0 pl-5 flex flex-col gap-1 text-sm text-text-secondary">
					<li>
						Insert a filament whose color you know and wait about 30
						seconds until the sensor color below stops changing.
					</li>
					<li>
						Enter the hex color of exactly that filament, you usually
						find it on the spool label or the manufacturer's page.
					</li>
					<li>
						Press "Add as reference", then repeat with the next
						filament. One or two references correct the white point.
						With three or more, ideally including a white or natural
						one, the device corrects the whole color.
					</li>
				</ol>
			</section>

			{/* Live sensor color */}
			<section class="flex flex-col gap-1.5">
				<span class="text-xs uppercase tracking-widest text-text-secondary font-500">
					Sensor color now
				</span>
				{measurement === "no_filament" ? (
					<div class="h-14 rounded-xl border border-border-secondary bg-bg-secondary flex items-center justify-center">
						<span class="text-xs text-text-secondary">No filament</span>
					</div>
				) : measurement === null ? (
					<div class="h-14 rounded-xl border border-border-secondary bg-bg-secondary flex items-center justify-center">
						<span class="text-xs text-text-secondary">Connecting…</span>
					</div>
				) : (
					<div
						class="h-14 rounded-xl border border-border-secondary transition-colors"
						style={{
							backgroundColor: measurement.hex_color
								? `#${measurement.hex_color}`
								: "transparent",
						}}
					/>
				)}
				{measurement !== null &&
					measurement !== "no_filament" &&
					measurement.hex_color && (
						<span class="font-mono text-xs text-text-secondary">
							#{measurement.hex_color.toUpperCase()}
						</span>
					)}
			</section>

			{/* Color of the inserted filament */}
			<section class="flex flex-col gap-2">
				<span class="text-xs uppercase tracking-widest text-text-secondary font-500">
					Color of the inserted filament (required)
				</span>
				<div class="flex mx-auto gap-3">
					<button
						class="w-10 h-10 rounded-xl border border-border-secondary shrink-0 transition-transform hover:scale-105"
						style={{
							backgroundColor: target ? hex(target) : "transparent",
						}}
						onClick={() => setShowPicker(true)}
						aria-label="Enter the filament color"
					/>
					<span class="font-mono text-sm text-text-secondary my-auto">
						{target ? hex(target) : "not entered"}
					</span>
					<div class="w-fit">
						<Button onClick={() => setShowPicker(true)}>
							{target ? "Change" : "Enter"}
						</Button>
					</div>
				</div>
			</section>

			{/* Actions */}
			<section class="flex flex-col gap-3">
				<Button disabled={!target || busy || full} onClick={addReference}>
					{busy ? "Adding…" : "Add as reference"}
				</Button>
				{!target && !busy && !full && (
					<p class="text-xs text-center text-text-secondary">
						Enter the color of the inserted filament first.
					</p>
				)}
				{message && (
					<p
						class={`text-xs text-center ${message.error ? "text-red-600" : "text-text-secondary"}`}
					>
						{message.text}
					</p>
				)}
			</section>

			{/* References */}
			<section class="flex flex-col gap-2">
				<span class="text-xs uppercase tracking-widest text-text-secondary font-500">
					Reference filaments ({references.length}/{MAX_REFERENCES})
				</span>
				<p class="m-0 text-sm text-text-secondary">
					{correctionMode(references.length)}
				</p>
				{references.map((ref, i) => (
					<div
						key={i}
						class="flex flex-wrap items-end gap-4 bg-bg-secondary rounded-xl px-3 py-2"
					>
						<Swatch color={ref.target} label="Spool" />
						<Swatch color={ref.corrected} label="Device" />
						<span class="text-xs text-text-secondary mb-1">
							ΔE {deltaE(ref.corrected, ref.target).toFixed(0)}
						</span>
						<button
							type="button"
							class="ml-auto mb-1 text-xs px-2 py-1 rounded border border-border-secondary bg-transparent text-text-secondary hover:cursor-pointer"
							onClick={() => removeReference(i)}
						>
							Remove
						</button>
					</div>
				))}
				{references.length > 0 && (
					<Button onClick={clearReferences}>Remove all references</Button>
				)}
			</section>

			{showPicker && (
				<ColorPickerModal
					initial={{
						r: target ? target[0] : 127,
						g: target ? target[1] : 127,
						b: target ? target[2] : 127,
					}}
					onApply={(r, g, b) => {
						setTarget([r, g, b]);
						setShowPicker(false);
					}}
					onClose={() => setShowPicker(false)}
				/>
			)}
		</>
	);
}
