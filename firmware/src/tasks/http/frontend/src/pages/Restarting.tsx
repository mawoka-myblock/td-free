export function RestartingPage() {
	return (
		<div class="flex flex-col">
			<h1 class="mx-auto">The Td-Free is restarting...</h1>
			<div class="mx-auto">
				<div
					class="inline-block h-12 w-12 animate-spin rounded-full border-4 border-solid border-current border-e-transparent align-[-0.125em] text-surface motion-reduce:animate-[spin_1.5s_linear_infinite] dark:text-white"
					role="status"
				>
					<span class="!absolute !-m-px !h-px !w-px !overflow-hidden !whitespace-nowrap !border-0 !p-0 ![clip:rect(0,0,0,0)]">
						Loading...
					</span>
				</div>
			</div>
			<p class="mx-auto">Please try reconnecting in around 20 seconds.</p>
		</div>
	);
}
