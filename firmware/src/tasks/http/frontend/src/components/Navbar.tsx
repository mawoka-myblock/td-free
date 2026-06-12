export function NavBar({ links = [], version }) {
	return (
		<footer class="mt-auto pt-4 border-t border-border flex flex-col gap-2">
			<nav class="flex justify-center gap-6">
				{links.map(({ href, label, hidden }) =>
					hidden ? null : (
						<a
							key={href}
							href={href}
							class="text-xs font-sans text-muted hover:text-accent transition-colors"
						>
							{label}
						</a>
					),
				)}
			</nav>
			{version && (
				<p class="text-center text-xs text-border font-mono">
					v{version}
				</p>
			)}
		</footer>
	);
}
