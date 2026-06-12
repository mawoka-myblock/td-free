import { render } from "preact";

import "virtual:uno.css";
import { DashboardPage } from "./pages/Dashboard";

export function App() {
	return <DashboardPage />;
}

function Resource(props) {
	return (
		<a href={props.href} target="_blank" class="resource">
			<h2>{props.title}</h2>
			<p>{props.description}</p>
		</a>
	);
}

render(<App />, document.getElementById("app"));
