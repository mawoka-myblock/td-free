import type { MeasurementData } from "../api";

type ChannelReadingsProps = Pick<MeasurementData, "td_r" | "td_g" | "td_b">;

export function ChannelReadings({ td_r, td_g, td_b }: ChannelReadingsProps) {
	const r = td_r != null ? parseFloat(td_r) : NaN;
	const g = td_g != null ? parseFloat(td_g) : NaN;
	const b = td_b != null ? parseFloat(td_b) : NaN;
	if (Number.isNaN(r) && Number.isNaN(g) && Number.isNaN(b)) {
		return null;
	}

	const values = [
		{ label: "R", value: r, color: "bg-red-500" },
		{ label: "G", value: g, color: "bg-green-500" },
		{ label: "B", value: b, color: "bg-blue-500" },
	];
	const max = Math.max(
		Number.isFinite(r) ? r : 0,
		Number.isFinite(g) ? g : 0,
		Number.isFinite(b) ? b : 0,
		0.01,
	);

	return (
		<div class="w-full flex flex-col gap-2 px-4">
			{values.map((c) => (
				<div key={c.label} class="flex items-center gap-2">
					<span class="font-mono text-sm w-4">{c.label}</span>
					<div class="flex-1 h-2 bg-slate-200 rounded-full overflow-hidden">
						{Number.isFinite(c.value) && (
							<div
								class={`h-full ${c.color} rounded-full`}
								style={{ width: `${Math.min(100, (c.value / max) * 100)}%` }}
							/>
						)}
					</div>
					<span class="font-mono text-xs w-10 text-right">
						{Number.isFinite(c.value) ? c.value.toFixed(1) : "—"}
					</span>
				</div>
			))}
		</div>
	);
}
