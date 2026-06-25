import { useState } from "preact/hooks";
import { Button } from "../components/Button";
import { Pages } from "./types";
import { ButtonLink } from "../components/ButtonLink";

export function WifiPage({ setPage }: { setPage: (page: Pages) => void }) {
	const [isValid, setIsValid] = useState(false);

	function updateValidity(e: Event) {
		const form = e.currentTarget as HTMLFormElement;
		setIsValid(form.checkValidity());
	}

	async function onSubmit(e: Event) {
		e.preventDefault();

		const form = e.currentTarget as HTMLFormElement;
		const formData = new FormData(form);
		console.log(formData);

		const ssid = formData.get("ssid")?.toString() ?? "";
		const password = formData.get("password")?.toString() ?? "";

		await fetch("/config/wifi", {
			method: "POST",
			headers: {
				"Content-Type": "application/json",
			},
			body: JSON.stringify({ ssid, password }),
		});

		setPage("restarting"); // optional redirect after success
	}

	return (
		<>
			<form
				class="flex flex-col gap-6"
				onInput={updateValidity}
				onSubmit={onSubmit}
			>
				<h1 class="mx-auto">Set WiFi Credentials</h1>

				<div class="flex flex-col mx-auto lg:w-2/3 w-full">
					<label htmlFor="ssid">Wifi SSID</label>
					<input
						name="ssid"
						type="text"
						id="ssid"
						required
						class="p-2 rounded shadow-lg invalid:border-red-400"
					></input>
				</div>
				<div class="flex flex-col mx-auto lg:w-2/3 w-full">
					<label htmlFor="password">Wifi Password</label>
					<input
						name="password"
						type="password"
						id="password"
						required
						class="p-2 rounded shadow-lg invalid:border-red-400"
					></input>
				</div>
				<div class="lg:w-2/3 w-full mx-auto">
					<Button type="submit" disabled={!isValid}>
						Submit
					</Button>
				</div>
			</form>
			<ButtonLink onClick={() => setPage("dashboard")}>
				Dashboard
			</ButtonLink>
		</>
	);
}
