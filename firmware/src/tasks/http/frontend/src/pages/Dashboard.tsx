import { useEffect, useState } from "preact/hooks";
import { ColorSwatch } from "../components/ColorSwatch";
import { ConfidenceBar } from "../components/ConfidenceBar";
import { NavBar } from "../components/Navbar";

export function DashboardPage() {
	const [spoolmanSaving, setSpoolmanSaving] = useState(false);
	const [measurement, setMeasurement] = useState<MeasurementChanged | null>(
		null,
		// {
		// 	td: 1.4,
		// 	hex_color: "FF0000",
		// 	buf_count: 21,
		// },
	);

	type MeasurementData = {
		td: string;
		hex_color?: string;
		buf_count?: number;
	};
	type MeasurementChanged = "no_filament" | MeasurementData;

	function handleSaveToSpoolman() {
		const id = prompt("Spoolman Filament ID");
		if (!id) return;
		if (!measurement?.td) {
			alert("No valid measurement");
			return;
		}
		window.location.assign(
			`/spoolman/set?filament_id=${id}&value=${measurement.td}`,
		);
	}

	useEffect(() => {
		const es = new EventSource("/events/data");

		const onMeasurementChanged = (event: MessageEvent) => {
			try {
				setMeasurement(JSON.parse(event.data));
			} catch {
				// Handles plain string payloads like: data: no_filament
				if (event.data === "no_filament") {
					setMeasurement("no_filament");
				}
			}
		};

		es.addEventListener(
			"measurement_changed",
			onMeasurementChanged as EventListener,
		);

		return () => {
			es.removeEventListener(
				"measurement_changed",
				onMeasurementChanged as EventListener,
			);
			es.close();
		};
	}, []);

	const navLinks = [
		{ href: "/wifi", label: "Wi-Fi" },
		{
			href: "/calibrate",
			label: "Calibrate",
			hidden: false,
		},
		{ href: "/settings", label: "Settings" },
	];

	return (
		<div class="min-h-screen flex items-center justify-center p-4 font-sans bg-[#4e6e58]">
			<div class="lg:w-1/3 w-5/6 h-4/5 flex flex-col gap-6 bg-black/60 p-4 rounded-xl shadow-lg">
				{/* Header */}
				<header class="flex justify-center h-fit">
					<h1 class="font-sans text-4xl font-600 text-text tracking-tight">
						Td-Free
					</h1>
				</header>

				{/* Measurement — the hero */}
				<div class="flex flex-col items-center gap-1 py-2">
					{/*<span class="label mb-1">Td</span>*/}
					<div
						class={`font-mono font-700 leading-none transition-colors text-4xl`}
					>
						{measurement?.td
							? (measurement.td ?? "—")
							: "No filament"}
					</div>
					{/*{!noFilament && !error && displayValue != null && (
						<span class="label mt-1">Td</span>
					)}*/}
				</div>

				{/* Color + Confidence */}
				{measurement?.hex_color && (
					<div class="flex flex-col gap-3 items-center">
						<ColorSwatch hex={`#${measurement?.hex_color}`} />
						<ConfidenceBar sampleCount={measurement?.buf_count} />
					</div>
				)}

				{/* Spoolman */}
				{/*{config.spoolman_available && value != null && (
					<button
						class="btn-primary w-full"
						onClick={handleSaveToSpoolman}
					>
						Save to Spoolman
					</button>
				)}*/}

				{/*<NavBar links={navLinks} version={config.version} />*/}
			</div>
		</div>
	);
}
