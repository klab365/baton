import type { LayoutLoad } from "./$types";

export const ssr = false;

export const load: LayoutLoad = async ({ fetch }) => {
	const response = await fetch("/api/project");
	if (!response.ok) throw new Error("Could not connect to the Baton backend.");
	let version: string | null = null;
	try {
		const versionResponse = await fetch("/api/version");
		if (versionResponse.ok) {
			const body: unknown = await versionResponse.json();
			if (
				typeof body === "object" &&
				body !== null &&
				"version" in body &&
				typeof body.version === "string"
			) {
				version = body.version;
			}
		}
	} catch {
		// A version lookup failure must not prevent the project from opening.
	}
	return { project: await response.json(), version };
};
