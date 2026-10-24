import { json } from "@sveltejs/kit";

/** Never forward upstream text/error bodies: they may contain sensitive details. */
export async function proxyProvider(
	request: Request,
	suffix: string,
	fetcher: typeof fetch,
) {
	try {
		const response = await fetcher(
			`${process.env.BATON_API_URL ?? "http://127.0.0.1:5173"}/api/providers${suffix}`,
			{
				method: request.method,
				headers: { "Content-Type": "application/json" },
				body: ["POST", "PUT"].includes(request.method)
					? await request.text()
					: undefined,
			},
		);
		if (!response.headers.get("content-type")?.includes("application/json")) {
			return json(
				{
					error:
						"Provider API unavailable. Start or restart the Baton Rust server and check BATON_API_URL.",
				},
				{ status: 502, headers: { "Cache-Control": "no-store" } },
			);
		}
		return new Response(response.body, {
			status: response.status,
			headers: {
				"Content-Type": "application/json",
				"Cache-Control": "no-store",
			},
		});
	} catch {
		return json(
			{
				error:
					"Cannot reach the provider API. Start the Baton Rust server and check BATON_API_URL.",
			},
			{ status: 502, headers: { "Cache-Control": "no-store" } },
		);
	}
}
