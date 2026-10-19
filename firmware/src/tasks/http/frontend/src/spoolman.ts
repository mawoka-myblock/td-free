export type Vendor = {
	id: number;
	registered: string;
	name: string;
	extra: Record<string, unknown>;
};

export type Filament = {
	id: number;
	registered: string;
	name: string;
	vendor: Vendor;
	material: string;
	price: number;
	density: number;
	diameter: number;
	weight: number;
	spool_weight: number;
	comment?: string;
	settings_extruder_temp?: number;
	settings_bed_temp?: number;
	color_hex: string;
	extra: { td?: string; [key: string]: unknown };
};

export type Settings = {
	led_brightness: number;
	algo: {
		b: number;
		m: number;
		threshold: number;
	};
	spoolman_host?: string;
	spoolman_port?: number;
};

export async function getSettings(): Promise<Settings> {
	if (import.meta.env.DEV) {
		await new Promise((resolve) => setTimeout(resolve, 300));
		return {
			led_brightness: 255,
			algo: {
				b: 0,
				m: 1,
				threshold: 0.9,
			},
			spoolman_host: "127.0.0.1",
			spoolman_port: 80,
		};
	}
	const res = await fetch("/config/settings");
	if (!res.ok) throw new Error("Failed to fetch settings");
	return await res.json();
}

export async function getFilaments(): Promise<Filament[]> {
	if (import.meta.env.DEV) {
		// Simulate the real endpoint's slowness so loading states are testable.
		await new Promise((resolve) => setTimeout(resolve, 800));
		return MOCK_FILAMENTS;
	}
	const res = await fetch("/spoolman/get-filaments");
	if (!res.ok) throw new Error("Failed to fetch filaments");
	return await res.json();
}

export async function setFilament(
	filamentId: number,
	td?: number,
	color?: string,
): Promise<boolean> {
	if (import.meta.env.DEV) {
		console.log("set filament, td:", td, "color:", color);
		await new Promise((resolve) => setTimeout(resolve, 400));
		const filament = MOCK_FILAMENTS.find((f) => f.id === filamentId);
		if (filament) filament.extra.td = String(td);
		return true;
	}
	const res = await fetch("/spoolman/set-filament", {
		method: "POST",
		headers: { "Content-Type": "application/json" },
		body: JSON.stringify({ filament_id: filamentId, td, color }),
	});
	return res.ok;
}

const MOCK_FILAMENTS: Filament[] = [
	{
		id: 1,
		registered: "2023-10-05T10:06:25Z",
		name: "Weiß PETG",
		vendor: {
			id: 3,
			registered: "2023-10-05T10:02:07Z",
			name: "Eryone",
			extra: {},
		},
		material: "PETG",
		price: 25.0,
		density: 1.27,
		diameter: 1.75,
		weight: 1000.0,
		spool_weight: 196.0,
		comment: "https://amzn.eu/d/6kJmkHQ",
		settings_extruder_temp: 250,
		settings_bed_temp: 80,
		color_hex: "FFFFFF",
		extra: {
			td: "4.3",
		},
	},
	{
		id: 2,
		registered: "2023-10-05T10:07:24Z",
		name: "Schwarz PLA",
		vendor: {
			id: 1,
			registered: "2023-10-05T10:00:29Z",
			name: "Geeetech",
			extra: {},
		},
		material: "PLA",
		price: 0.0,
		density: 1.24,
		diameter: 1.75,
		weight: 1000.0,
		spool_weight: 181.0,
		comment: "https://amzn.eu/d/8RREca2",
		settings_bed_temp: 60,
		color_hex: "000000",
		extra: {},
	},
	{
		id: 3,
		registered: "2023-10-05T10:09:19Z",
		name: "Grün PLA",
		vendor: {
			id: 2,
			registered: "2023-10-05T10:00:53Z",
			name: "Overture",
			extra: {},
		},
		material: "PLA+",
		price: 20.0,
		density: 1.24,
		diameter: 1.75,
		weight: 1000.0,
		spool_weight: 169.0,
		comment: "https://amzn.eu/d/72iTw9Y",
		settings_extruder_temp: 210,
		settings_bed_temp: 60,
		color_hex: "22A700",
		extra: {},
	},
];
