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
	spoolman_host?: string;
	spoolman_port?: number;
};

function round(value: number, decimals = 6) {
	const factor = Math.pow(10, decimals);
	return Math.round(value * factor) / factor;
}

export function SettingsPage({ setPage }: { setPage: (page: Pages) => void }) {
	const [isValid, setIsValid] = useState(true);
	const [loading, setLoading] = useState(true);
	const [version, setVersion] = useState("");

	const [settings, setSettings] = useState<Settings>({
		led_brightness: 100,
		algo: {
			b: 0,
			m: 1,
			threshold: 0.9,
		},
		spoolman_host: undefined,
	});

	useEffect(() => {
		let cancelled = false;

		(async () => {
			try {
				const res = await fetch("/config/settings");
				if (!res.ok) throw new Error("Failed to load settings");

				const data = (await res.json()) as Settings;

				if (!cancelled) {
					setSettings({
						led_brightness: round(data.led_brightness, 2),
						algo: {
							b: round(data.algo.b),
							m: round(data.algo.m),
							threshold: round(data.algo.threshold, 3),
						},
						spoolman_host: !data.spoolman_host
							? ""
							: data.spoolman_host,
						spoolman_port: !data.spoolman_port
							? 0
							: data.spoolman_port,
					});
				}
			} catch (err) {
				console.error(err);
			} finally {
				if (!cancelled) setLoading(false);
			}
			try {
				const infoRes = await fetch("/config/info");
				if (infoRes.ok) {
					const info = (await infoRes.json()) as { version: string };
					if (!cancelled) setVersion(info.version);
				}
			} catch (err) {
				console.error(err);
			}
		})();

		return () => {
			cancelled = true;
		};
	}, []);

	function updateSpoolmanValidity() {
		const ip = document.getElementById("spoolman_ip") as HTMLInputElement;
		const port = document.getElementById(
			"spoolman_port",
		) as HTMLInputElement;

		const ipSet = ip.value !== "";
		const portSet = port.value !== "";

		if (ipSet !== portSet) {
			const message =
				"IP and port must either both be set or both be empty";
			port.setCustomValidity(message);
		} else {
			port.setCustomValidity(
				/^(?:\d{2,}).?$/.test(port.value) ? "" : "Not matching format",
			);
		}
	}

	function updateValidity(e: Event) {
		updateSpoolmanValidity();

		const form = e.currentTarget as HTMLFormElement;
		setIsValid(form.checkValidity());
	}

	function updateField<T>(path: string[], value: T) {
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
			spoolman_host:
				!settings.spoolman_host || settings.spoolman_host === "" // empty or undefined/null
					? undefined
					: settings.spoolman_host,
			spoolman_port:
				!settings.spoolman_port || settings.spoolman_port === 0
					? undefined
					: settings.spoolman_port,
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
						step={1}
						value={settings.led_brightness}
						onInput={(e) =>
							updateField(
								["led_brightness"],
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

				{/* Spoolman URL */}
				<div class="flex flex-col mx-auto lg:w-2/3 w-full">
					<label htmlFor="spoolman_ip">Spoolman IP</label>
					<input
						id="spoolman_ip"
						type="text"
						value={settings.spoolman_host}
						onInput={(e) => {
							updateField(
								["spoolman_host"],
								(e.target as HTMLInputElement).value.replace(
									/[^0-9.]/g,
									"",
								),
							);
						}}
						class="p-2 rounded shadow-lg invalid:border-red-400"
					/>
				</div>

				{/* Spoolman Port */}
				<div class="flex flex-col mx-auto lg:w-2/3 w-full">
					<label htmlFor="spoolman_port">Spoolman Port</label>
					<input
						id="spoolman_port"
						type="text"
						value={
							settings.spoolman_port
								? settings.spoolman_port.toString()
								: ""
						}
						onInput={(e) => {
							let val = (e.target as HTMLInputElement).value
								.replace(/[^0-9]/g, "")
								.slice(0, 5);
							updateField(
								["spoolman_port"],
								val === "" ? undefined : Number(val),
							);
						}}
						class="p-2 rounded shadow-lg invalid:border-red-400"
					/>
				</div>
				<p class="text-white mx-auto lg:w-2/3 w-full text-center -my-2">
					Note:
					<br />
					The Spoolman integration only supports http (unencrypted)
					traffic.
					<br />
					Only IP addresses are allowed.
					<br />
					You have to add a custom field of type <b>float</b> under
					Settings-Extra Fields-Filaments. The key must be <b>td</b>.
				</p>
				<div class="lg:w-2/3 w-full mx-auto">
					<Button type="submit" disabled={!isValid}>
						Save
					</Button>
				</div>
			</form>

			<ButtonLink onClick={() => setPage("dashboard")}>
				Dashboard
			</ButtonLink>

			{version && (
				<p class="text-white text-center text-sm -my-2">
					Firmware v{version}
				</p>
			)}
		</>
	);
}
