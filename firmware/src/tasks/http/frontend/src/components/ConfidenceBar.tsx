/** Animated confidence bar. sampleCount out of maxSamples. */
export function ConfidenceBar({
	sampleCount,
	maxSamples = 100,
}: {
	sampleCount: number;
	maxSamples?: number;
}) {
	const pct = Math.min(100, (sampleCount / maxSamples) * 100);

	return (
		<div style={{ textAlign: "center", fontSize: 12, color: "#64748b" }}>
			<div style={{ marginBottom: 4 }}>
				Confidence: <strong>{pct.toFixed(0)}%</strong>
			</div>
			<div
				style={{
					width: 200,
					height: 6,
					background: "#e2e8f0",
					borderRadius: 3,
					overflow: "hidden",
					margin: "0 auto",
				}}
			>
				<div
					style={{
						height: "100%",
						width: `${pct}%`,
						background:
							"linear-gradient(to right, #f87171, #fbbf24, #4ade80)",
						borderRadius: 3,
						transition: "width 0.3s ease",
					}}
				/>
			</div>
		</div>
	);
}
