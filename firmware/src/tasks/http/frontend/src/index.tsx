import { render } from "preact";

import "virtual:uno.css";
import { DashboardPage } from "./pages/Dashboard";

import { useState } from "preact/hooks";
import { Pages } from "./pages/types";
import { WifiPage } from "./pages/Wifi";
import { RestartingPage } from "./pages/Restarting";
import { SettingsPage } from "./pages/Settings";
import { CalibrationPage } from "./pages/Calibrate";

export function App() {
	const [page, setPage] = useState<Pages>("dashboard");

	return (
		<div class="min-h-screen flex items-center justify-center p-4 font-sans bg-[#4e6e58]">
			<div class="lg:w-1/3 w-5/6 h-4/5 flex flex-col gap-6 bg-black/60 p-4 rounded-xl shadow-lg">
				{page === "dashboard" && <DashboardPage setPage={setPage} />}
				{page === "settings" && <SettingsPage setPage={setPage} />}
				{page === "calibrate" && <CalibrationPage setPage={setPage} />}
				{page === "wifi" && <WifiPage setPage={setPage} />}
				{page === "restarting" && <RestartingPage />}
			</div>
		</div>
	);
}

render(<App />, document.getElementById("app"));
