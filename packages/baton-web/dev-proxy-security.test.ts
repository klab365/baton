import { describe, expect, test } from "bun:test";
import { IncomingMessage } from "node:http";
import { Socket } from "node:net";
import { allowedDevProxyRequest } from "./dev-proxy-security";

function request(headers: string[], method = "POST", remote = "127.0.0.1") {
	const socket = new Socket();
	Object.defineProperties(socket, {
		localPort: { value: 5174 },
		remoteAddress: { value: remote },
	});
	const req = new IncomingMessage(socket);
	req.method = method;
	req.rawHeaders = headers;
	return allowedDevProxyRequest(req);
}

describe("development API proxy boundary", () => {
	test("accepts same-origin browser and origin-less local clients", () => {
		expect(
			request([
				"Host",
				"127.0.0.1:5174",
				"Origin",
				"http://127.0.0.1:5174",
				"Sec-Fetch-Site",
				"same-origin",
			]),
		).toBe(true);
		expect(request(["Host", "localhost:5174"])).toBe(true);
	});
	test("rejects foreign form posts, rebinding, duplicates and remote clients", () => {
		for (const headers of [
			["Host", "127.0.0.1:5174", "Origin", "http://evil.test"],
			["Host", "evil.test:5174"],
			["Host", "127.0.0.1:9999"],
			["Host", "127.0.0.1:5174", "Origin", "null"],
			[
				"Host",
				"127.0.0.1:5174",
				"Origin",
				"http://127.0.0.1:5174",
				"Origin",
				"http://evil.test",
			],
			["Host", "127.0.0.1:5174", "Sec-Fetch-Site", "cross-site"],
		])
			expect(request(headers)).toBe(false);
		expect(request(["Host", "localhost:5174"], "POST", "192.0.2.1")).toBe(
			false,
		);
	});
});
