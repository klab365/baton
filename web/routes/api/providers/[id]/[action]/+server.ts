import { proxyProvider } from "$lib/server/provider-proxy";
import type { RequestHandler } from "./$types";
const proxy: RequestHandler = ({ request, params, fetch }) => {
	const suffix =
		params.action === "provider" ? "" : `/${encodeURIComponent(params.action)}`;
	return proxyProvider(
		request,
		`/${encodeURIComponent(params.id)}${suffix}`,
		fetch,
	);
};
export const GET = proxy;
export const POST = proxy;
export const PUT = proxy;
export const DELETE = proxy;
