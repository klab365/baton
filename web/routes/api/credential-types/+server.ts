import type { RequestHandler } from "./$types";

export const GET: RequestHandler = async ({ fetch }) => {
	const response = await fetch(
		`${process.env.BATON_API_URL ?? "http://127.0.0.1:5173"}/api/credential-types`,
	);
	return new Response(response.body, {
		status: response.status,
		headers: {
			"Content-Type": response.headers.get("Content-Type") ?? "text/plain",
		},
	});
};
