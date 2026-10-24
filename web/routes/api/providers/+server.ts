import { proxyProvider } from "$lib/server/provider-proxy";
import type { RequestHandler } from "./$types";
const proxy: RequestHandler = ({ request, fetch }) =>
	proxyProvider(request, "", fetch);
export const GET = proxy;
export const POST = proxy;
