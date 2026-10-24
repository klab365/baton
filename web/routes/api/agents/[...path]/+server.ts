import type { RequestHandler } from "./$types";

const proxy: RequestHandler = async ({ request, params, fetch }) => {
	const suffix = params.path ? `/${encodeURIComponent(params.path)}` : "";
	const response = await fetch(
		`${process.env.BATON_API_URL ?? "http://127.0.0.1:5173"}/api/agents${suffix}`,
		{
			method: request.method,
			headers: { "Content-Type": "application/json" },
			body: ["POST", "PUT", "DELETE"].includes(request.method)
				? await request.text()
				: undefined,
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
export const PUT = proxy;
export const DELETE = proxy;
