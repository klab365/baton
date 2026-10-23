import type { LayoutServerLoad } from "./$types";

const apiUrl = process.env.BATON_API_URL ?? "http://127.0.0.1:5173";

export const load: LayoutServerLoad = async ({ fetch }) => {
	const response = await fetch(`${apiUrl}/api/project`);
	if (!response.ok) throw new Error("Could not connect to the Baton backend.");
	return { project: await response.json() };
};
