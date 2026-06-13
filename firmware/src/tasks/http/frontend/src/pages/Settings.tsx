import { useEffect, useState } from "preact/hooks";
import { Button } from "../components/Button";
import { Pages } from "./types";
import { ButtonLink } from "../components/ButtonLink";

type Settings = {
	led_brightness: number;
	algo: {
		b: number;
		m: number;
		threshold: number;
	};
};

function round(value: number, decimals = 6) {
	const factor = Math.pow(10, decimals);
	return Math.round(value * factor) / factor;
}

export function SettingsPage({ setPage }: { setPage: (page: Pages) => void }) {
	const [isValid, setIsValid] = useState(true);
	const [loading, setLoading] = useState(true);

	const [settings, setSettings] = useState<Settings>({
		led_brightness: 100,
		algo: {
			b: 0,
			m: 1,
			threshold: 0.9,
		},
	});

	useEffect(() => {
		let cancelled = false;

		(async () => {
			try {
				const res = await fetch("/config/settings");
				if (!res.ok) throw new Error("Failed to load settings");

				const data = (await res.json()) as Settings;

				if (!cancelled) {
					setSettings(data);
				}
			} catch (err) {
				console.error(err);
			} finally {
				if (!cancelled) setLoading(false);
			}
		})();

		return () => {
			cancelled = true;
		};
	}, []);

	function updateValidity(e: Event) {
		const form = e.currentTarget as HTMLFormElement;
		setIsValid(form.checkValidity());
	}

	function updateField(path: string[], value: number) {
		setSettings((prev) => {
			const copy = structuredClone(prev);

			let ref: any = copy;
			for (let i = 0; i < path.length - 1; i++) {
				ref = ref[path[i]];
			}

			ref[path[path.length - 1]] = value;
			return copy;
		});
	}

	async function onSubmit(e: Event) {
		e.preventDefault();

		const payload: Settings = {
			led_brightness: round(settings.led_brightness, 2),
			algo: {
				b: round(settings.algo.b),
				m: round(settings.algo.m),
				threshold: round(settings.algo.threshold, 6),
			},
		};

		await fetch("/config/settings", {
			method: "POST",
			headers: { "Content-Type": "application/json" },
			body: JSON.stringify(payload),
		});

		setPage("dashboard");
	}

	if (loading) {
		return <div class="mx-auto">Loading settings…</div>;
	}

	return (
		<>
			<form
				class="flex flex-col gap-6"
				onInput={updateValidity}
				onSubmit={onSubmit}
			>
				<h1 class="mx-auto">Settings</h1>

				{/* LED brightness */}
				<div class="flex flex-col mx-auto lg:w-2/3 w-full">
					<label htmlFor="led">LED Brightness (%)</label>
					<input
						type="number"
						min={0}
						max={100}
						value={settings.led_brightness}
						onInput={(e) =>
							updateField(
								[],
								Number((e.target as HTMLInputElement).value),
							)
						}
						class="p-2 rounded shadow-lg"
					/>
				</div>

				{/* b */}
				<div class="flex flex-col mx-auto lg:w-2/3 w-full">
					<label htmlFor="b">Algo b</label>
					<input
						type="number"
						step="0.01"
						value={settings.algo.b}
						onInput={(e) =>
							updateField(
								["algo", "b"],
								Number((e.target as HTMLInputElement).value),
							)
						}
						class="p-2 rounded shadow-lg"
					/>
				</div>

				{/* m */}
				<div class="flex flex-col mx-auto lg:w-2/3 w-full">
					<label htmlFor="m">Algo m</label>
					<input
						type="number"
						step="0.01"
						value={settings.algo.m}
						onInput={(e) =>
							updateField(
								["algo", "m"],
								Number((e.target as HTMLInputElement).value),
							)
						}
						class="p-2 rounded shadow-lg"
					/>
				</div>

				{/* threshold */}
				<div class="flex flex-col mx-auto lg:w-2/3 w-full">
					<label htmlFor="threshold">Threshold (0.001 - 0.999)</label>
					<input
						type="number"
						step="0.001"
						min={0.001}
						max={0.999}
						value={settings.algo.threshold}
						onInput={(e) =>
							updateField(
								["algo", "threshold"],
								Number((e.target as HTMLInputElement).value),
							)
						}
						class="p-2 rounded shadow-lg invalid:border-red-400"
					/>
				</div>

				<div class="lg:w-2/3 w-full mx-auto">
					<Button type="submit" disabled={!isValid}>
						Save
					</Button>
				</div>
			</form>

			<ButtonLink onClick={() => setPage("dashboard")}>
				Dashboard
			</ButtonLink>
		</>
	);
}
