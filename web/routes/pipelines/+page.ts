import type { PageLoad } from "./$types";
export const load: PageLoad = async ({ fetch, parent }) => {
	const { project } = await parent();
	if (project.status !== "ready") return { configuration: null };
	const response = await fetch("/api/pipelines");
	if (!response.ok) throw new Error("Could not load pipeline configuration.");
	return { configuration: await response.json() };
};
