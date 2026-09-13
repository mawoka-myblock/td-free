import { useEffect, useState } from "preact/hooks";
import { ChannelReadings } from "../components/ChannelReadings";
import { ColorSwatch } from "../components/ColorSwatch";
import { ConfidenceBar } from "../components/ConfidenceBar";
import { Pages } from "./types";
import { ButtonLink } from "../components/ButtonLink";
import { MeasurementChanged } from "../api";

export function DashboardPage({ setPage }: { setPage: (page: Pages) => void }) {
	const [measurement, setMeasurement] = useState<MeasurementChanged | null>(
		import.meta.env.PROD
			? null
			: 				{
					td: "1.4",
					td_r: "0.8",
					td_g: "1.5",
					td_b: "0.9",
					hex_color: "FF0000",
					buf_count: 21,
				},
	);

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

	return (
		<>
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
					{measurement?.td ? (measurement.td ?? "—") : "No filament"}
				</div>
			</div>

			{/* Color + Confidence */}
			{measurement?.hex_color && (
				<div class="flex flex-col gap-3 items-center">
					<ColorSwatch hex={`#${measurement.hex_color}`} />
					<ConfidenceBar sampleCount={measurement.buf_count} />
				</div>
			)}
			{typeof measurement === "object" &&
				measurement != null &&
				(measurement.td_r || measurement.td_g || measurement.td_b) && (
					<ChannelReadings
						td_r={measurement.td_r}
						td_g={measurement.td_g}
						td_b={measurement.td_b}
					/>
				)}
			<div class="flex justify-around">
				<ButtonLink onClick={() => setPage("settings")}>
					Settings
				</ButtonLink>
				<ButtonLink onClick={() => setPage("wifi")}>Wifi</ButtonLink>
				<ButtonLink onClick={() => setPage("calibrate")}>
					Calibrate
				</ButtonLink>
			</div>
		</>
	);
}
