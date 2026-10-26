import type { RequestHandler } from "./$types";

const proxy: RequestHandler = async ({ params, fetch, request }) => {
	const suffix = params.path
		? `/${params.path.split("/").map(encodeURIComponent).join("/")}`
		: "";
	const response = await fetch(
		`${process.env.BATON_API_URL ?? "http://127.0.0.1:5173"}/api/runtime-profiles${suffix}`,
		{
			method: request.method,
			headers: { "Content-Type": "application/json" },
			body: ["PUT", "DELETE"].includes(request.method)
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
export const PUT = proxy;
export const DELETE = proxy;
