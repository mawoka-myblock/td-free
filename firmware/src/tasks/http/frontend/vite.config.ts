import { defineConfig } from "vite";
import preact from "@preact/preset-vite";
import UnoCSS from "unocss/vite";
import { compression } from "vite-plugin-compression2";

// https://vitejs.dev/config/
export default defineConfig({
	plugins: [
		preact(),
		UnoCSS(),
		compression({
			algorithms: ["gzip"],
			deleteOriginalAssets: true,
		}),
	],
	build: {
		cssCodeSplit: false,
		rollupOptions: {
			output: {
				entryFileNames: "app.js",
				chunkFileNames: "app.js",
				assetFileNames: (ai) => {
					const names = ai.names ?? [];
					if (names.some((n) => n.endsWith(".css"))) {
						return "app.css";
					}
					return "asset.[ext]";
				},
			},
		},
	},
});
