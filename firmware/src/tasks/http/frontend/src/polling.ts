import { useEffect, useRef } from "preact/hooks";

/**
 * Calls `fn` in a loop with at least `minInterval` ms between starts.
 * Cleans up on unmount.
 */
export function usePolling(
	fn: () => void | Promise<void>,
	minInterval: number = 1500,
	delay: number = 300,
): void {
	const fnRef = useRef<() => void | Promise<void>>(fn);

	// keep latest function without retriggering effect
	fnRef.current = fn;

	useEffect(() => {
		let active = true;
		let timeoutId: ReturnType<typeof setTimeout>;

		async function loop() {
			if (!active) return;

			const start = Date.now();

			try {
				await fnRef.current();
			} catch (e) {
				console.warn("Polling error:", e);
			}

			if (!active) return;

			const elapsed = Date.now() - start;
			const wait = Math.max(minInterval - elapsed, 0);

			timeoutId = setTimeout(loop, wait);
		}

		timeoutId = setTimeout(loop, delay);

		return () => {
			active = false;
			clearTimeout(timeoutId);
		};
	}, []); // intentionally empty: fn is handled via ref
}
