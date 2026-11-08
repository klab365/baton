import adapter from "@sveltejs/adapter-static";

/** @type {import('@sveltejs/kit').Config} */
const config = {
	kit: {
		adapter: adapter({ fallback: "index.html" }),
		alias: {
			$components: "components",
		},
		files: {
			lib: "lib",
			routes: "routes",
			appTemplate: "app.html",
		},
	},
};

export default config;
