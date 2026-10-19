import { useEffect, useMemo, useState } from "preact/hooks";
import { Filament, getFilaments, setFilament } from "../spoolman";
import { Button } from "./Button";

type Status =
	| { kind: "loading" }
	| { kind: "error"; message: string }
	| { kind: "ready" };

function normalizeHex(hex: string) {
	return hex.trim().toLowerCase().replace(/^#/, "");
}

export function FilamentPicker({
	td,
	color,
	onSaved,
}: {
	td: string;
	color: string;
	onSaved?: () => void;
}) {
	const [status, setStatus] = useState<Status>({ kind: "loading" });
	const [filaments, setFilaments] = useState<Filament[]>([]);
	const [query, setQuery] = useState("");
	const [pending, setPending] = useState<Filament | null>(null); // needs confirm
	const [savingId, setSavingId] = useState<number | null>(null);
	const [saveError, setSaveError] = useState<string | null>(null);
	const [saveTd, setSaveTd] = useState(true);
	const [saveColor, setSaveColor] = useState(false);

	const nothingSelected = !saveTd && !saveColor;

	useEffect(() => {
		let cancelled = false;
		getFilaments()
			.then((data) => {
				if (cancelled) return;
				setFilaments(data);
				setStatus({ kind: "ready" });
			})
			.catch((err) => {
				if (cancelled) return;
				setStatus({
					kind: "error",
					message:
						err instanceof Error ? err.message : "Unknown error",
				});
			});
		return () => {
			cancelled = true;
		};
	}, []);

	const filtered = useMemo(() => {
		const q = query.trim().toLowerCase();
		if (!q) return filaments;
		return filaments.filter((f) =>
			[f.name, f.vendor?.name, f.material, f.color_hex, String(f.id)]
				.filter(Boolean)
				.some((field) => field!.toLowerCase().includes(q)),
		);
	}, [filaments, query]);

	// Which of the selected fields actually need a confirmation for this
	// filament (i.e. would overwrite an existing, different value).
	const confirmReasons = (filament: Filament) => {
		const reasons: { td?: string; color?: string } = {};
		if (saveTd && filament.extra?.td) {
			reasons.td = filament.extra.td;
		}
		if (
			saveColor &&
			filament.color_hex &&
			normalizeHex(filament.color_hex) !== normalizeHex(color)
		) {
			reasons.color = filament.color_hex;
		}
		return reasons;
	};

	const doSave = async (filament: Filament) => {
		setPending(null);
		setSaveError(null);
		setSavingId(filament.id);
		try {
			const tdValue = saveTd ? parseFloat(td) : undefined;
			const colorValue = saveColor ? color : undefined;
			const ok = await setFilament(filament.id, tdValue, colorValue);
			if (ok) {
				setFilaments((prev) =>
					prev.map((f) =>
						f.id === filament.id
							? {
									...f,
									extra:
										tdValue !== undefined
											? {
													...f.extra,
													td: String(tdValue),
												}
											: f.extra,
									color_hex:
										colorValue !== undefined
											? colorValue
											: f.color_hex,
								}
							: f,
					),
				);
				onSaved?.();
			} else {
				setSaveError("Server rejected the save.");
				setSavingId(null);
			}
		} catch (err) {
			setSaveError(err instanceof Error ? err.message : "Failed to save");
			setSavingId(null);
		}
	};

	const handlePick = (filament: Filament) => {
		setSaveError(null);
		const reasons = confirmReasons(filament);
		if (reasons.td || reasons.color) {
			setPending(filament);
		} else {
			doSave(filament);
		}
	};

	// --- Confirmation view (replaces the list while a filament with an
	// existing td and/or a differing color is pending confirmation) ---
	if (pending) {
		const reasons = confirmReasons(pending);
		return (
			<div class="flex flex-col gap-4 items-center text-center py-4">
				<div class="flex items-center gap-2">
					<div
						class="w-10 h-10 rounded-full border border-text/20"
						style={{ backgroundColor: `#${pending.color_hex}` }}
					/>
					{reasons.color && (
						<>
							<span class="text-text/40">→</span>
							<div
								class="w-10 h-10 rounded-full border border-text/20"
								style={{ backgroundColor: `#${color}` }}
							/>
						</>
					)}
				</div>

				<div class="font-sans text-sm text-text flex flex-col gap-1">
					<span class="font-600">{pending.name}</span> already has{" "}
					{reasons.td && reasons.color
						? "a TD value and a different color"
						: reasons.td
							? "a TD value"
							: "a different color"}{" "}
					set.
				</div>

				<div class="font-sans text-sm text-text/70">
					{reasons.td && (
						<div>
							Overwrite td{" "}
							<span class="font-mono text-text">
								{reasons.td}
							</span>{" "}
							with <span class="font-mono text-text">{td}</span>
						</div>
					)}
					{reasons.color && (
						<div>
							Overwrite color{" "}
							<span
								class="inline-block w-3 h-3 rounded-full border border-text/20 align-middle"
								style={{ backgroundColor: `#${reasons.color}` }}
							/>{" "}
							with{" "}
							<span
								class="inline-block w-3 h-3 rounded-full border border-text/20 align-middle"
								style={{ backgroundColor: `#${color}` }}
							/>
						</div>
					)}
				</div>

				<div class="flex gap-3 mt-1">
					<Button type="button" onClick={() => setPending(null)}>
						Cancel
					</Button>
					<Button type="button" onClick={() => doSave(pending)}>
						Overwrite
					</Button>
				</div>
			</div>
		);
	}

	if (status.kind === "loading") {
		return (
			<div class="flex h-full">
				<div
					class="m-auto inline-block h-12 w-12 animate-spin rounded-full border-4 border-solid border-current border-e-transparent align-[-0.125em] text-surface motion-reduce:animate-[spin_1.5s_linear_infinite] dark:text-white"
					role="status"
				>
					<span class="!absolute !-m-px !h-px !w-px !overflow-hidden !whitespace-nowrap !border-0 !p-0 ![clip:rect(0,0,0,0)]">
						Loading...
					</span>
				</div>
			</div>
		);
	}

	if (status.kind === "error") {
		return (
			<div class="text-sm text-red-500 font-sans py-4 text-center">
				Failed to load filaments: {status.message}
			</div>
		);
	}

	return (
		<div class="flex flex-col gap-3 w-full">
			{/* Save-target toggle */}
			<div class="flex gap-2 justify-center">
				<button
					type="button"
					onClick={() => setSaveTd((v) => !v)}
					class={`flex items-center gap-2 px-3 py-1.5 rounded-lg text-sm font-sans border transition-colors cursor-pointer ${
						saveTd
							? "bg-[#4e6e58] border-[#4e6e58] text-white"
							: "bg-transparent border-white/40 text-text/70"
					}`}
				>
					<span class="font-mono">TD {td}</span>
				</button>
				<button
					type="button"
					onClick={() => setSaveColor((v) => !v)}
					class={`flex items-center gap-2 px-3 py-1.5 rounded-lg text-sm font-sans border transition-colors cursor-pointer ${
						saveColor
							? "bg-[#4e6e58] border-[#4e6e58] text-white"
							: "bg-transparent border-white/40 text-text/70"
					}`}
				>
					<span
						class="inline-block w-3 h-3 rounded-full border border-white/40"
						style={{ backgroundColor: `#${color}` }}
					/>
					Color
				</button>
			</div>
			<p class="text-center -my-1">
				Toggle the buttons above to choose what to save.
			</p>

			{nothingSelected && (
				<div class="text-sm text-amber-500 font-sans text-center">
					Select at least TD or Color to save.
				</div>
			)}

			<input
				type="text"
				value={query}
				onInput={(e) => setQuery((e.target as HTMLInputElement).value)}
				placeholder="Search filament..."
				autoFocus
				class="rounded-lg border-2 border-white bg-transparent px-3 py-2 text-sm font-sans text-text outline-none focus:border-[#4e6e58] transition-all"
			/>

			{saveError && (
				<div class="text-sm text-red-500 font-sans">{saveError}</div>
			)}

			<div class="flex flex-col divide-y divide-text/10 -mx-1 gap-4">
				{filtered.length === 0 && (
					<div class="flex h-full">
						<div class="m-auto flex flex-col">
							<span class="text-8xl font-bold mx-auto">!</span>
							<div class="mx-auto">No matches</div>
						</div>
					</div>
				)}

				{filtered.map((f) => {
					const isSaving = savingId === f.id;
					return (
						<button
							key={f.id}
							type="button"
							onClick={() => handlePick(f)}
							disabled={savingId !== null || nothingSelected}
							class="flex items-center gap-3 w-full text-left p-2 rounded-lg hover:bg-text/5 disabled:opacity-40 transition-colors border-none cursor-pointer bg-[#4e6e58]/30 hover:bg-[#4e6e58]/70"
						>
							<span
								class="inline-block w-4 h-4 rounded-full border border-text/20 shrink-0"
								style={{ backgroundColor: `#${f.color_hex}` }}
							/>
							<span class="flex-1 min-w-0">
								<span class="block text-sm font-sans text-text truncate">
									{f.vendor?.name
										? `${f.vendor.name} - `
										: ""}
									{f.name}
								</span>
								<span class="block text-xs font-sans text-text/40">
									{f.material}
								</span>
							</span>
							<span class="text-xs font-mono text-text/50 shrink-0">
								{isSaving
									? "Saving…"
									: f.extra?.td
										? `TD ${f.extra.td}`
										: "no TD"}
							</span>
						</button>
					);
				})}
			</div>
		</div>
	);
}
