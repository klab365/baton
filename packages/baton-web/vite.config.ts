import tailwindcss from "@tailwindcss/vite";
import { sveltekit } from "@sveltejs/kit/vite";
import { defineConfig } from "vite";
import { allowedDevProxyRequest } from "./dev-proxy-security";

const target = process.env.BATON_API_URL ?? "http://127.0.0.1:5173";

export default defineConfig({
	plugins: [
		{
			name: "secure-dev-api-proxy",
			configureServer(server) {
				server.middlewares.use("/api", (req, res, next) => {
					if (!allowedDevProxyRequest(req)) {
						res.writeHead(403).end();
						return;
					}
					next();
				});
			},
		},
		tailwindcss(),
		sveltekit(),
	],
	server: {
		watch: { ignored: ["**/.baton/**"] },
		proxy: {
			"/api": {
				target,
				changeOrigin: true,
				configure(proxy) {
					proxy.on("proxyReq", (proxyReq, req) => {
						if (req.headers.origin)
							proxyReq.setHeader("origin", new URL(target).origin);
					});
				},
			},
		},
	},
	optimizeDeps: {
		exclude: ["bits-ui", "@lucide/svelte"],
	},
});
