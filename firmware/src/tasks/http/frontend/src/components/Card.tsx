import type { ComponentChildren } from "preact";

export function Card({
	children,
	style = {},
}: {
	children: ComponentChildren;
	style: object;
}) {
	return (
		<div
			style={{
				background: "#fff",
				boxShadow: "0 4px 24px rgba(0,0,0,0.08)",
				borderRadius: 14,
				padding: "1.5rem",
				width: "90%",
				maxWidth: 480,
				margin: "0 auto",
				display: "flex",
				flexDirection: "column",
				gap: "1rem",
				...style,
			}}
		>
			{children}
		</div>
	);
}

/** Page-level heading inside a card */
export function CardTitle({ children }) {
	return (
		<h1
			style={{
				margin: 0,
				textAlign: "center",
				fontSize: "1.25rem",
				fontWeight: 600,
				color: "#1e293b",
				letterSpacing: "-0.01em",
			}}
		>
			{children}
		</h1>
	);
}

/** Subtle divider */
export function Divider() {
	return (
		<hr
			style={{
				border: "none",
				borderTop: "1px solid #f1f5f9",
				margin: "0.25rem 0",
			}}
		/>
	);
}
