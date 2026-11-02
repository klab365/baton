import type { LayoutLoad } from "./$types";

export const ssr = false;

export const load: LayoutLoad = async ({ fetch }) => {
	const response = await fetch("/api/project");
	if (!response.ok) throw new Error("Could not connect to the Baton backend.");
	return { project: await response.json() };
};
