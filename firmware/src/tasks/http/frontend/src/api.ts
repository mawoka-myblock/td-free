/**
 * api.ts — all backend communication in one place.
 * Swap endpoint paths here when the server routes change.
 */

const BASE = "";

// ── Types ─────────────────────────────────────────────────────────────────────

export type Config = {
	version: string;
	spoolman_available: boolean;
	color_available: boolean;
};

export type EmptyMeasurement = {
	type: "empty";
};

export type NoFilamentMeasurement = {
	type: "no_filament";
};

export type ValueMeasurementWithColor = {
	type: "value";
	td: number;
	hex: string;
	confidence: number;
};

export type ValueMeasurement = {
	type: "value";
	td: number;
};

export type MeasurementData = {
	td: string;
	hex_color?: string;
	buf_count?: number;
};
export type MeasurementChanged = "no_filament" | MeasurementData;

export type Measurement =
	| EmptyMeasurement
	| NoFilamentMeasurement
	| ValueMeasurement
	| ValueMeasurementWithColor;

export type RgbMultipliers = {
	red: number;
	green: number;
	blue: number;
	brightness: number;
	td_reference: number;
	reference_r: number;
	reference_g: number;
	reference_b: number;
	rgb_disabled: boolean;
};

export type RgbMultipliersInput = {
	red: number;
	green: number;
	blue: number;
	brightness: number;
	td_reference: number;
	reference_r: number;
	reference_g: number;
	reference_b: number;
};

export type AutoCalibrateInput = {
	reference_r: number;
	reference_g: number;
	reference_b: number;
};

export type AutoCalibrateResponse = {
	status: "success" | "error";
	message?: string;
	red?: number;
	green?: number;
	blue?: number;
	brightness?: number;
	td_reference?: number;
};

// ── Config ────────────────────────────────────────────────────────────────────

export async function fetchConfig(): Promise<Config> {
	const res = await fetch(`${BASE}/config`);
	if (!res.ok) throw new Error(`Config fetch failed: ${res.status}`);
	return res.json();
}

// ── Measurement ───────────────────────────────────────────────────────────────

/**
 * Fetches the latest sensor reading.
 */
export async function fetchMeasurement(): Promise<Measurement> {
	const res = await fetch(`${BASE}/fallback`);
	if (!res.ok) throw new Error(`Measurement fetch failed: ${res.status}`);

	const text = await res.text();

	if (!text) return { type: "empty" };
	if (text === "no_filament") return { type: "no_filament" };

	const parts = text.split(",");
	const td = parseFloat(parts[0]);

	if (parts.length >= 2 && parts[1].startsWith("#")) {
		const confidence = parts.length >= 3 ? parseInt(parts[2], 10) || 0 : 0;

		return {
			type: "value",
			td,
			hex: parts[1],
			confidence,
		};
	}

	return {
		type: "value",
		td,
	};
}

// ── RGB Multipliers ───────────────────────────────────────────────────────────

export async function fetchRgbMultipliers(): Promise<RgbMultipliers> {
	const res = await fetch(`${BASE}/rgb_multipliers`);
	if (!res.ok) throw new Error(`RGB fetch failed: ${res.status}`);
	return res.json();
}

export async function saveRgbMultipliers(
	mult: RgbMultipliersInput,
): Promise<void> {
	const res = await fetch(`${BASE}/rgb_multipliers`, {
		method: "POST",
		headers: { "Content-Type": "application/json" },
		body: JSON.stringify(mult),
	});

	if (!res.ok) throw new Error(`RGB save failed: ${res.status}`);
}

// ── Auto-calibrate ────────────────────────────────────────────────────────────

export async function autoCalibrate(
	ref: AutoCalibrateInput,
): Promise<AutoCalibrateResponse> {
	const res = await fetch(`${BASE}/auto_calibrate`, {
		method: "POST",
		headers: { "Content-Type": "application/json" },
		body: JSON.stringify(ref),
	});

	if (!res.ok) throw new Error(`Calibration failed: ${res.status}`);

	const data: AutoCalibrateResponse = await res.json();

	if (data.status !== "success") {
		throw new Error(data.message || "Unknown error");
	}

	return data;
}

// ── Spoolman ──────────────────────────────────────────────────────────────────

export function buildSpoolmanUrl(params: {
	filament_id: string;
	value: number;
}): string {
	return `${BASE}/spoolman/set?filament_id=${encodeURIComponent(
		params.filament_id,
	)}&value=${params.value}`;
}
