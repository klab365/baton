import type { IncomingMessage } from "node:http";

export function allowedDevProxyRequest(req: IncomingMessage): boolean {
	const values = (name: string) => {
		const result: string[] = [];
		for (let i = 0; i < req.rawHeaders.length; i += 2) {
			if (req.rawHeaders[i].toLowerCase() === name)
				result.push(req.rawHeaders[i + 1]);
		}
		return result;
	};
	if (
		!["127.0.0.1", "::1", "::ffff:127.0.0.1"].includes(
			req.socket.remoteAddress ?? "",
		)
	)
		return false;
	const hosts = values("host");
	const port = req.socket.localPort;
	if (
		hosts.length !== 1 ||
		!port ||
		![`localhost:${port}`, `127.0.0.1:${port}`, `[::1]:${port}`].includes(
			hosts[0].toLowerCase(),
		)
	)
		return false;
	if (["GET", "HEAD", "OPTIONS"].includes(req.method ?? "")) return true;
	const origins = values("origin");
	if (
		origins.length > 1 ||
		(origins.length === 1 && origins[0] !== `http://${hosts[0]}`)
	)
		return false;
	const sites = values("sec-fetch-site");
	return (
		sites.length <= 1 &&
		(sites.length === 0 || sites[0] === "same-origin" || sites[0] === "none")
	);
}
