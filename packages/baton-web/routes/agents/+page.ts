import type { PageLoad } from "./$types";

export const load: PageLoad = async ({ fetch, parent }) => {
	const { project } = await parent();
	if (project.status !== "ready") return { configuration: null };
	const [response, credentialsResponse, typesResponse] = await Promise.all([
		fetch("/api/agents"),
		fetch("/api/credentials"),
		fetch("/api/credential-types"),
	]);
	if (!response.ok || !credentialsResponse.ok || !typesResponse.ok)
		throw new Error("Could not load agent configuration.");
	return {
		configuration: await response.json(),
		credentials: await credentialsResponse.json(),
		credentialTypes: await typesResponse.json(),
	};
};
