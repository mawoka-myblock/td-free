import { useEffect, useState } from "preact/hooks";
import { Button } from "./Button";
import { FilamentPicker } from "./FilamentPicker";
import { Modal } from "./Modal";
import { getSettings, Settings } from "../spoolman";

export function SpoolmanSave({ td, color }: { td?: string, color?: string }) {
	const [settings, setSettings] = useState<Settings | null>(null);
	const [open, setOpen] = useState(false);

	useEffect(() => {
		getSettings()
			.then(setSettings)
			.catch(() => setSettings(null));
	}, []);

	if (!settings?.spoolman_host || !td)
		return (
			<>
				<div class="w-1 h-9" />
			</>
		);

	return (
		<>
			<div class="flex justify-center w-full">
				<div>
					<Button onClick={() => setOpen(true)}>
						Save to Spoolman
					</Button>
				</div>
			</div>

			{open && (
				<Modal
					title={`Select filament for the TD of ${td}`}
					onClose={() => setOpen(false)}
				>
					<FilamentPicker td={td} color={color} onSaved={() => setOpen(false)} />
				</Modal>
			)}
		</>
	);
}
