import type { PageLoad } from "./$types";
export const load: PageLoad = async ({ fetch, parent }) => {
	const { project } = await parent();
	if (project.status !== "ready") return { configuration: null };
	const response = await fetch("/api/skills");
	if (!response.ok) throw new Error("Could not load skills.");
	return { configuration: await response.json() };
};
