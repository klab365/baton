import type { RequestHandler } from "./$types";

const proxy: RequestHandler = async ({ request, params, fetch }) => {
	const suffix = params.path
		? `/${params.path.split("/").map(encodeURIComponent).join("/")}`
		: "";
	const response = await fetch(
		`${process.env.BATON_API_URL ?? "http://127.0.0.1:5173"}/api/runs${suffix}`,
		{
			method: request.method,
			headers: { "Content-Type": "application/json" },
			body: request.method === "POST" ? await request.text() : undefined,
		},
	);
	return new Response(response.body, {
		status: response.status,
		headers: {
			"Content-Type": response.headers.get("Content-Type") ?? "text/plain",
		},
	});
};
export const GET = proxy;
export const POST = proxy;
