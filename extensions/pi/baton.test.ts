import { expect, test } from "bun:test";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import baton from "./baton";
import { readFile } from "node:fs/promises";
import { resolve } from "node:path";
import net from "node:net";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { choosePipeline, drive, manage, herdrRequest } from "./baton";

function ui(
	overrides: Partial<Parameters<typeof drive>[2]> = {},
): Parameters<typeof drive>[2] {
	return {
		select: async () => undefined,
		editor: async () => undefined,
		confirm: async () => false,
		notify: () => {},
		setStatus: () => {},
		...overrides,
	};
}

test("sends ordered working and idle Herdr JSON-RPC reports with pane routing", async () => {
	const dir = await mkdtemp(join(tmpdir(), "baton-herdr-"));
	const socketPath = join(dir, "herdr.sock");
	const requests: Record<string, any>[] = [];
	let received!: () => void;
	const complete = new Promise<void>((resolve) => { received = resolve; });
	const server = net.createServer((socket) => {
		let data = "";
		socket.on("data", (chunk) => { data += chunk.toString(); });
		socket.on("end", () => {
			const line = data.trimEnd();
			requests.push(JSON.parse(line));
			if (requests.length === 2) received();
		});
	});
	await new Promise<void>((resolve) => server.listen(socketPath, resolve));
	const old = { env: process.env.HERDR_ENV, path: process.env.HERDR_SOCKET_PATH, pane: process.env.HERDR_PANE_ID };
	try {
		process.env.HERDR_ENV = "1";
		process.env.HERDR_SOCKET_PATH = socketPath;
		process.env.HERDR_PANE_ID = "pane-test";
		herdrRequest("pane.report_agent", { source: "baton:pi", agent: "pi", state: "working", seq: 10 });
		herdrRequest("pane.report_agent", { source: "baton:pi", agent: "pi", state: "idle", seq: 11 });
		await complete;
		expect(requests.map((request) => request.params.state)).toEqual(["working", "idle"]);
		expect(requests.map((request) => request.params.seq)).toEqual([10, 11]);
		for (const request of requests) {
			expect(request.method).toBe("pane.report_agent");
			expect(request.params.pane_id).toBe("pane-test");
			expect(request.params.source).toBe("baton:pi");
		}
	} finally {
		if (old.env === undefined) delete process.env.HERDR_ENV; else process.env.HERDR_ENV = old.env;
		if (old.path === undefined) delete process.env.HERDR_SOCKET_PATH; else process.env.HERDR_SOCKET_PATH = old.path;
		if (old.pane === undefined) delete process.env.HERDR_PANE_ID; else process.env.HERDR_PANE_ID = old.pane;
		await new Promise<void>((resolve) => server.close(() => resolve()));
		await rm(dir, { recursive: true, force: true });
	}
});

test("Herdr requests are disabled without the complete environment and tolerate unavailable sockets", () => {
	const old = { env: process.env.HERDR_ENV, path: process.env.HERDR_SOCKET_PATH, pane: process.env.HERDR_PANE_ID };
	try {
		delete process.env.HERDR_ENV;
		delete process.env.HERDR_SOCKET_PATH;
		delete process.env.HERDR_PANE_ID;
		expect(() => herdrRequest("notification.show", { title: "Baton" })).not.toThrow();
		process.env.HERDR_ENV = "1";
		process.env.HERDR_SOCKET_PATH = join(tmpdir(), `missing-herdr-${Date.now()}.sock`);
		process.env.HERDR_PANE_ID = "pane-test";
		expect(() => herdrRequest("notification.show", { title: "Baton" })).not.toThrow();
		process.env.HERDR_SOCKET_PATH = "\\0invalid";
		expect(() => herdrRequest("notification.show", { title: "Baton" })).not.toThrow();
	} finally {
		if (old.env === undefined) delete process.env.HERDR_ENV; else process.env.HERDR_ENV = old.env;
		if (old.path === undefined) delete process.env.HERDR_SOCKET_PATH; else process.env.HERDR_SOCKET_PATH = old.path;
		if (old.pane === undefined) delete process.env.HERDR_PANE_ID; else process.env.HERDR_PANE_ID = old.pane;
	}
});

test("selects newest attempts by global attempt number, not stored placeholder order", async () => {
	const messages: string[] = [];
	const steps = Array.from({ length: 40 }, (_, index) => ({
		stepId: `step-${index}`, attempt: 40 - index, status: "succeeded",
	}));
	steps[0] = { stepId: "latest", attempt: 41, status: "running" };
	steps[1] = { stepId: "pending-placeholder", attempt: 0, status: "pending" };
	await manage("inspect run-order", async () => ({ id: "run-order", status: "running", steps }), ui(), (text) => messages.push(text));
	expect(messages[0]).toContain("latest");
	expect(messages[0]).toContain('"stepId": "step-10"');
	expect(messages[0]).not.toContain('"stepId": "step-39"');
	expect(messages[0]).not.toContain("pending-placeholder");
});

test("bounds large cost summaries and preserves run identity and attempt details", async () => {
	const messages: string[] = [];
	const steps = Array.from({ length: 40 }, (_, attempt) => ({
		stepId: `step-${attempt}`, attempt: attempt + 1, status: "succeeded", durationMs: 1,
		usage: { estimatedCost: "0.01", priceSource: "x".repeat(1000), inputTokens: 1 },
	}));
	await manage("inspect run-long", async () => ({ id: "run-long", status: "succeeded", steps }), ui(), (text) => messages.push(text));
	expect(messages[0].length).toBeLessThanOrEqual(8000);
	expect(messages[0]).toContain("run-long");
	expect(messages[0]).toContain("step-39");
	expect(messages[0]).toContain("omittedEarlierSteps");
});

test("Pi inspection totals only complete comparable cost observations", async () => {
	const messages: string[] = [];
	await manage("inspect run-cost", async () => ({ id: "run-cost", steps: [
		{ stepId: "a", attempt: 1, status: "succeeded", usage: { estimatedCost: "0.10", currency: "USD", priceUnit: "perMillionTokens", coverage: "complete" } },
		{ stepId: "b", attempt: 2, status: "succeeded", usage: { estimatedCost: "0.20", currency: "USD", priceUnit: "perMillionTokens", coverage: "complete" } },
	] }), ui(), (text) => messages.push(text));
	expect(messages[0]).toContain("Run cost: 0.3 USD");
});

test("Pi inspection rejects totals with mixed price units", async () => {
	const messages: string[] = [];
	await manage("inspect run-mixed", async () => ({ id: "run-mixed", steps: [
		{ stepId: "a", attempt: 1, status: "succeeded", usage: { estimatedCost: "1", currency: "USD", priceUnit: "perToken", coverage: "complete" } },
		{ stepId: "b", attempt: 2, status: "succeeded", usage: { estimatedCost: "1", currency: "USD", priceUnit: "perMillionTokens", coverage: "complete" } },
	] }), ui(), (text) => messages.push(text));
	expect(messages[0]).toContain("Run cost: unavailable");
});

test("Pi inspection summarizes persisted metrics without treating missing values as zero", async () => {
	const messages: string[] = [];
	await manage("inspect run-1", async () => ({ id: "run-1", steps: [
		{ stepId: "build", attempt: 1, status: "succeeded", durationMs: 0, usage: { inputTokens: 0, outputTokens: 4, cacheReadTokens: 2, estimatedCost: "0.01", currency: "USD", priceUnit: "perMillionTokens", coverage: "complete", priceSnapshot: { input: 1 }, priceSource: "catalog", priceCheckedAt: "now" } },
		{ stepId: "build", attempt: 2, status: "failed", durationMs: null, selectedTransition: { to: "build" }, usage: { inputTokens: 1 } },
		{ stepId: "finish", attempt: 0, status: "pending" },
	] }), ui(), (text) => messages.push(text));
	expect(messages[0]).toContain("Attempts: 2 · retries: 1");
	expect(messages[0]).toContain("1 (complete) / 4 (partial) / 2 (partial) / unavailable (unavailable)");
	expect(messages[0]).toContain("Run cost: unavailable (currency/unit not recorded)");
	expect(messages[0]).toContain("Attempt costs: 0.01 · catalog · checked now");
	expect(messages[0]).toContain('"currency": "USD"');
	expect(messages[0]).toContain('"priceUnit": "perMillionTokens"');
	expect(messages[0]).toContain('"coverage": "complete"');
	expect(messages[0]).toContain("Tokens input/output/cache read/cache write: 1 (complete) / 4 (partial) / 2 (partial) / unavailable (unavailable)");
	expect(messages[0]).toContain("Duration: 0 ms (partial)");
});

test("shipped single-file Pi extension has no project-relative runtime imports", async () => {
	const source = await readFile(resolve(import.meta.dir, "baton.ts"), "utf8");
	expect(source).not.toMatch(/from [\"']\.\.?\//);
});

test("lists and inspects persisted work sessions without executing them", async () => {
	const calls: string[][] = [];
	const messages: string[] = [];
	const exec = async (args: string[]) => {
		calls.push(args);
		return args.length === 1 ? [{ id: "run-1", status: "running" }] : { id: "run-1", steps: [] };
	};
	expect(await manage("list", exec, ui(), (text) => messages.push(text))).toBe(true);
	expect(await manage("inspect run-1", exec, ui(), (text) => messages.push(text))).toBe(true);
	expect(calls).toEqual([["status"], ["status", "--run", "run-1"]]);
	expect(messages.join("\\n")).toContain("Work sessions");
	expect(messages.join("\\n")).toContain("run-1");
});

test("paginates session lists so records beyond the first page remain accessible", async () => {
	const messages: string[] = [];
	const runs = Array.from({ length: 55 }, (_, i) => ({ id: `run-${i}`, status: "succeeded" }));
	await manage("list 2", async () => runs, ui(), (text) => messages.push(text));
	expect(messages[0]).toContain("run-50");
	expect(messages[0]).toContain("Page 2 of 2");
});

test("keeps long session records reachable across bounded list pages", async () => {
	const messages: string[] = [];
	const runs = Array.from({ length: 60 }, (_, i) => ({ id: `run-${i}`, status: "succeeded", currentStep: `step-${i}-${"x".repeat(500)}` }));
	for (let page = 1; page <= 10; page++) {
		await manage(`list ${page}`, async () => runs, ui(), (text) => messages.push(text));
		const pageCount = Number(messages.at(-1)?.match(/Page \d+ of (\d+)/)?.[1]);
		if (page >= pageCount) break;
	}
	const text = messages.join("\\n");
	expect(text.length).toBeLessThan(8500 * 10);
	expect(Array.from(text.matchAll(/"id": "(run-\d+)"/g), (match) => match[1]).sort()).toEqual(runs.map((run) => run.id).sort());
	expect(text).toContain("currentStep truncated");
});

test("marks omitted work sessions and inspection data when output is bounded", async () => {
	const messages: string[] = [];
	const runs = Array.from({ length: 55 }, (_, i) => ({ id: `run-${i}`, status: "succeeded" }));
	await manage("list", async () => runs, ui(), (text) => messages.push(text));
	expect(messages[0]).toContain("Page 1 of 2");
	expect(messages[0]).toContain("next: /baton list 2");
	messages.length = 0;
	await manage("inspect run-1", async () => ({
		id: "run-1", steps: Array.from({ length: 32 }, (_, i) => ({ stepId: `step-${i}`, output: "x".repeat(1200) })),
	}), ui(), (text) => messages.push(text));
	expect(messages[0]).toContain("Work session run-1");
	expect(messages[0]).toContain("2 earlier steps omitted");
	expect(messages[0]).toContain("output truncated");
});

test("resumes a selected work session through the existing driver", async () => {
	const calls: string[][] = [];
	await manage("resume run-1", async (args) => {
		calls.push(args);
		return { status: "succeeded" };
	}, ui(), () => {});
	expect(calls).toEqual([["next", "--run", "run-1"], ["status", "--run", "run-1"]]);
});

test("refines a completed session through a new run in the same worktree", async () => {
	const calls: string[][] = [];
	const messages: string[] = [];
	const result = await manage(
		"refine",
		async (args) => {
			calls.push(args);
			if (args.length === 1) return [
				{ id: "older-run", status: "succeeded", pipelineId: "software-factory", startedAt: "2026-01-01T00:00:00Z" },
				{ id: "latest-run", status: "succeeded", pipelineId: "software-factory", startedAt: "2026-02-01T00:00:00Z" },
			];
			if (args[0] === "refine") return { id: "new-run", status: "running" };
			return { id: "new-run", status: "succeeded" };
		},
		ui({
			select: async (_prompt, options) => {
				expect(options[0]).toContain("latest-run");
				expect(options[0]).toContain("★");
				return options[0];
			},
			editor: async () => "Fix the remaining edge case",
		}),
		(text) => messages.push(text),
	);
	expect(result).toBe(true);
	expect(calls).toEqual([
		["status"],
		["refine", "--run", "latest-run", "--instruction", "Fix the remaining edge case"],
		["next", "--run", "new-run"],
		["status", "--run", "new-run"],
	]);
	expect(messages.join("\n")).toContain("Baton refinement started: new-run (from latest-run)");
});

test("documents command side effects in help and rejects malformed commands", async () => {
	const messages: string[] = [];
	expect(await manage("help", async () => [], ui(), (text) => messages.push(text))).toBe(true);
	expect(messages[0]).toContain("list [PAGE]");
	expect(messages[0]).toContain("ask for human decisions");
	await expect(manage("inspect", async () => [], ui(), () => {})).rejects.toThrow("Usage:");
});

test("cancelling the run picker or refinement editor never starts a run", async () => {
	const pickerCalls: string[][] = [];
	await manage("refine", async (args) => {
		pickerCalls.push(args);
		return args.length === 1 ? [{ id: "done", status: "succeeded" }] : { id: "new-run" };
	}, ui({ select: async () => undefined }), () => {});
	expect(pickerCalls).toEqual([["status"]]);

	const editorCalls: string[][] = [];
	await manage("refine done", async (args) => {
		editorCalls.push(args);
		return args.length === 1 ? [{ id: "done", status: "succeeded" }] : { id: "new-run" };
	}, ui({ editor: async () => undefined }), () => {});
	expect(editorCalls).toEqual([["status"]]);
});

test("refine rejects explicitly selected runs that are not completed", async () => {
	const calls: string[][] = [];
	await expect(manage("refine failed", async (args) => {
		calls.push(args);
		return [{ id: "failed", status: "failed" }];
	}, ui(), () => {})).rejects.toThrow("No completed work session with ID 'failed'");
	expect(calls).toEqual([["status"]]);
});

test("read-only Baton inspect reports activity without sending a notification", async () => {
	const dir = await mkdtemp(join(tmpdir(), "baton-command-herdr-"));
	const socketPath = join(dir, "herdr.sock");
	const requests: any[] = [];
	let gotAll!: () => void;
	const all = new Promise<void>((resolve) => { gotAll = resolve; });
	const server = net.createServer((socket) => {
		let data = "";
		socket.on("data", (chunk) => { data += chunk.toString(); });
		socket.on("end", () => {
			requests.push(JSON.parse(data.trimEnd()));
			if (requests.length === 2) gotAll();
		});
	});
	await new Promise<void>((resolve) => server.listen(socketPath, resolve));
	const old = { env: process.env.HERDR_ENV, path: process.env.HERDR_SOCKET_PATH, pane: process.env.HERDR_PANE_ID };
	let handler!: (args: string, ctx: any) => Promise<void>;
	const pi = {
		on: () => () => {},
		registerCommand: (_name: string, options: { handler: typeof handler }) => { handler = options.handler; },
		exec: async () => ({ code: 0, killed: false, truncated: false, stderr: "", stdout: JSON.stringify({ id: "run-secret", status: "succeeded", task: "secret task", steps: [] }) }),
		sendMessage: () => {},
	} as unknown as ExtensionAPI;
	try {
		process.env.HERDR_ENV = "1";
		process.env.HERDR_SOCKET_PATH = socketPath;
		process.env.HERDR_PANE_ID = "pane-test";
		baton(pi);
		await handler("inspect run-secret", { hasUI: true, cwd: "/tmp", ui: { setStatus() {}, notify() {} } });
		await all;
		expect(requests.map((r) => r.method)).toEqual(["pane.report_agent", "pane.report_agent"]);
		expect(requests.map((r) => r.params.state)).toEqual(["working", "idle"]);
		expect(requests[0].params.seq).toBeLessThan(requests[1].params.seq);
	} finally {
		if (old.env === undefined) delete process.env.HERDR_ENV; else process.env.HERDR_ENV = old.env;
		if (old.path === undefined) delete process.env.HERDR_SOCKET_PATH; else process.env.HERDR_SOCKET_PATH = old.path;
		if (old.pane === undefined) delete process.env.HERDR_PANE_ID; else process.env.HERDR_PANE_ID = old.pane;
		await new Promise<void>((resolve) => server.close(() => resolve()));
		await rm(dir, { recursive: true, force: true });
	}
});

test("registered Baton command reports idle after CLI failure", async () => {
	const dir = await mkdtemp(join(tmpdir(), "baton-command-error-"));
	const socketPath = join(dir, "herdr.sock");
	const states: string[] = [];
	let gotBoth!: () => void;
	const both = new Promise<void>((resolve) => { gotBoth = resolve; });
	const server = net.createServer((socket) => {
		let data = "";
		socket.on("data", (chunk) => { data += chunk.toString(); });
		socket.on("end", () => {
			states.push(JSON.parse(data.trimEnd()).params.state);
			if (states.length === 2) gotBoth();
		});
	});
	await new Promise<void>((resolve) => server.listen(socketPath, resolve));
	const old = { env: process.env.HERDR_ENV, path: process.env.HERDR_SOCKET_PATH, pane: process.env.HERDR_PANE_ID };
	let handler!: (args: string, ctx: any) => Promise<void>;
	const pi = {
		on: () => () => {},
		registerCommand: (_name: string, options: { handler: typeof handler }) => { handler = options.handler; },
		exec: async () => ({ code: 1, killed: false, truncated: false, stderr: "failed", stdout: "" }),
		sendMessage: () => {},
	} as unknown as ExtensionAPI;
	try {
		process.env.HERDR_ENV = "1"; process.env.HERDR_SOCKET_PATH = socketPath; process.env.HERDR_PANE_ID = "pane-test";
		baton(pi);
		await handler("inspect run-error", { hasUI: true, cwd: "/tmp", ui: { setStatus() {}, notify() {} } });
		await both;
		expect(states).toEqual(["working", "idle"]);
	} finally {
		if (old.env === undefined) delete process.env.HERDR_ENV; else process.env.HERDR_ENV = old.env;
		if (old.path === undefined) delete process.env.HERDR_SOCKET_PATH; else process.env.HERDR_SOCKET_PATH = old.path;
		if (old.pane === undefined) delete process.env.HERDR_PANE_ID; else process.env.HERDR_PANE_ID = old.pane;
		await new Promise<void>((resolve) => server.close(() => resolve()));
		await rm(dir, { recursive: true, force: true });
	}
});

test("does not re-notify an unchanged human pause across repeated resumes", async () => {
	const dir = await mkdtemp(join(tmpdir(), "baton-resume-herdr-"));
	const socketPath = join(dir, "herdr.sock");
	const requests: any[] = [];
	let allReceived!: () => void;
	const all = new Promise<void>((resolve) => { allReceived = resolve; });
	const server = net.createServer((socket) => {
		let data = "";
		socket.on("data", (chunk) => { data += chunk.toString(); });
		socket.on("end", () => { requests.push(JSON.parse(data.trimEnd())); if (requests.length === 5) allReceived(); });
	});
	await new Promise<void>((resolve) => server.listen(socketPath, resolve));
	const old = { env: process.env.HERDR_ENV, path: process.env.HERDR_SOCKET_PATH, pane: process.env.HERDR_PANE_ID };
	let handler!: (args: string, ctx: any) => Promise<void>;
	const pi = {
		on: () => () => {},
		registerCommand: (_name: string, options: { handler: typeof handler }) => { handler = options.handler; },
		exec: async (_command: string, args: string[]) => ({ code: 0, killed: false, truncated: false, stderr: "", stdout: JSON.stringify({ runId: "run-1", revision: 1, status: "waitingHuman", step: { id: "review", name: "Review", human: true }, attempt: 1, allowedOutcomes: ["accept"], lastResult: null }) }),
		sendMessage: () => {},
	} as unknown as ExtensionAPI;
	try {
		process.env.HERDR_ENV = "1"; process.env.HERDR_SOCKET_PATH = socketPath; process.env.HERDR_PANE_ID = "pane-test";
		baton(pi);
		const ctx = { hasUI: true, cwd: "/tmp", ui: { setStatus() {}, notify() {}, select: async () => undefined } };
		await handler("resume run-1", ctx);
		await handler("resume run-1", ctx);
		await all;
		expect(requests.filter((r) => r.method === "notification.show")).toHaveLength(1);
	} finally {
		if (old.env === undefined) delete process.env.HERDR_ENV; else process.env.HERDR_ENV = old.env;
		if (old.path === undefined) delete process.env.HERDR_SOCKET_PATH; else process.env.HERDR_SOCKET_PATH = old.path;
		if (old.pane === undefined) delete process.env.HERDR_PANE_ID; else process.env.HERDR_PANE_ID = old.pane;
		await new Promise<void>((resolve) => server.close(() => resolve()));
		await rm(dir, { recursive: true, force: true });
	}
});

test("notifies once for completed ordinary assistant responses, not tool events", async () => {
	const dir = await mkdtemp(join(tmpdir(), "baton-message-herdr-"));
	const socketPath = join(dir, "herdr.sock");
	const requests: any[] = [];
	let received!: () => void;
	const done = new Promise<void>((resolve) => { received = resolve; });
	const server = net.createServer((socket) => {
		let data = "";
		socket.on("data", (chunk) => { data += chunk.toString(); });
		socket.on("end", () => { requests.push(JSON.parse(data.trimEnd())); if (requests.length === 3) received(); });
	});
	await new Promise<void>((resolve) => server.listen(socketPath, resolve));
	const old = { env: process.env.HERDR_ENV, path: process.env.HERDR_SOCKET_PATH, pane: process.env.HERDR_PANE_ID };
	const handlers = new Map<string, (event: any) => Promise<void>>();
	const pi = { on: (name: string, handler: (event: any) => Promise<void>) => { handlers.set(name, handler); }, registerCommand() {} } as unknown as ExtensionAPI;
	try {
		process.env.HERDR_ENV = "1"; process.env.HERDR_SOCKET_PATH = socketPath; process.env.HERDR_PANE_ID = "pane-test";
		baton(pi);
		const ctx = { ui: { setStatus() {} } };
		await handlers.get("agent_start")!({}, ctx);
		const onMessageEnd = handlers.get("message_end")!;
		await onMessageEnd({ message: { role: "assistant", stopReason: "toolUse", content: "tool" } });
		await onMessageEnd({ message: { role: "assistant", stopReason: "stop", content: "private answer" } });
		await handlers.get("agent_end")!({}, ctx);
		await done;
		expect(requests.map((request) => request.params.state).filter(Boolean)).toEqual(["working", "idle"]);
		expect(requests.filter((request) => request.method === "notification.show")).toHaveLength(1);
		expect(requests.find((request) => request.method === "notification.show").params.body).not.toContain("private answer");
	} finally {
		if (old.env === undefined) delete process.env.HERDR_ENV; else process.env.HERDR_ENV = old.env;
		if (old.path === undefined) delete process.env.HERDR_SOCKET_PATH; else process.env.HERDR_SOCKET_PATH = old.path;
		if (old.pane === undefined) delete process.env.HERDR_PANE_ID; else process.env.HERDR_PANE_ID = old.pane;
		await new Promise<void>((resolve) => server.close(() => resolve()));
		await rm(dir, { recursive: true, force: true });
	}
});

test("registered inspect command routes to work status", async () => {
	let handler: (args: string, ctx: any) => Promise<void> = async () => {};
	const calls: string[][] = [];
	const output: string[] = [];
	const pi = {
		on: () => () => {},
		registerCommand: (_name: string, options: { handler: typeof handler }) => { handler = options.handler; },
		exec: async (_command: string, args: string[]) => {
			calls.push(args);
			return { code: 0, killed: false, truncated: false, stderr: "", stdout: JSON.stringify({ id: "run-1", status: "running", task: "do not expose", configSnapshot: { secret: "hidden" }, steps: [{ stepId: "build", status: "succeeded", output: "x".repeat(5000) }] }) };
		},
		sendMessage: ({ content }: { content: string }) => { output.push(content); },
	} as unknown as ExtensionAPI;
	baton(pi);
	await handler("inspect run-1", {
		hasUI: true, cwd: "/tmp", ui: { setStatus() {}, notify() {} },
	} as any);
	expect(calls).toEqual([["run", "status", "--run", "run-1"]]);
	expect(output.join(" ")).toContain("run-1");
	expect(output.join(" ")).not.toContain("do not expose");
	expect(output.join(" ")).not.toContain("hidden");
	expect(output.join(" ").length).toBeLessThan(8000);
});

test("registered refine command starts the follow-up from the current Pi worktree", async () => {
	let handler: (args: string, ctx: any) => Promise<void> = async () => {};
	const calls: string[][] = [];
	const pi = {
		on: () => () => {},
		registerCommand: (_name: string, options: { handler: typeof handler }) => { handler = options.handler; },
		exec: async (_command: string, args: string[]) => {
			calls.push(args);
			const value = args[1] === "status"
				? [{ id: "old-run", status: "succeeded" }]
				: args[1] === "refine"
					? { id: "new-run", status: "running" }
					: { id: "new-run", status: "succeeded" };
			return { code: 0, killed: false, truncated: false, stderr: "", stdout: JSON.stringify(value) };
		},
		sendMessage: () => {},
	} as unknown as ExtensionAPI;
	baton(pi);
	await handler("refine old-run", {
		hasUI: true,
		cwd: "/tmp/worktree",
		ui: {
			select: async (_prompt: string, options: string[]) => options[0],
			editor: async () => "Check the CI failure and fix it",
			setStatus() {},
			notify() {},
		},
	} as any);
	expect(calls).toEqual([
		["run", "status"],
		["run", "refine", "--run", "old-run", "--instruction", "Check the CI failure and fix it"],
		["run", "next", "--run", "new-run"],
		["run", "status", "--run", "new-run"],
	]);
});

test("animates ordinary Pi agent turns and clears status when the turn ends", async () => {
	const handlers = new Map<string, (event: any, ctx: any) => Promise<void>>();
	const statuses: (string | undefined)[] = [];
	const pi = {
		on: (name: string, handler: (event: any, ctx: any) => Promise<void>) => { handlers.set(name, handler); },
		registerCommand: () => {},
	} as unknown as ExtensionAPI;
	baton(pi);
	const ctx = { ui: { setStatus: (_key: string, value?: string) => statuses.push(value) } };
	await handlers.get("agent_start")!({ type: "agent_start" }, ctx);
	await new Promise((resolve) => setTimeout(resolve, 220));
	await handlers.get("agent_end")!({ type: "agent_end", messages: [] }, ctx);
	expect(statuses[0]).toContain("Pi · working");
	expect(new Set(statuses.filter((status) => status?.includes("Pi · working"))).size).toBeGreaterThan(1);
	expect(statuses.at(-1)).toBeUndefined();
});

test("chooses a pipeline from the keyboard-selectable list", async () => {
	const calls: string[][] = [];
	const selected = await choosePipeline(
		async (args) => {
			calls.push(args);
			return ["factory", "review"];
		},
		ui({ select: async (_prompt, options) => options[1] }),
	);
	expect(calls).toEqual([["pipelines"]]);
	expect(selected).toBe("review");
});

test("pipeline selection cancellation and empty configuration do not select", async () => {
	const warnings: string[] = [];
	expect(
		await choosePipeline(async () => ["factory"], ui({ select: async () => undefined })),
	).toBeUndefined();
	expect(
		await choosePipeline(
			async () => [],
			ui({ notify: (message) => warnings.push(message) }),
		),
	).toBeUndefined();
	expect(warnings).toEqual(["Baton: no pipelines are configured"]);
});

test("surfaces pipeline listing errors", async () => {
	await expect(choosePipeline(async () => { throw new Error("invalid config"); }, ui())).rejects.toThrow("invalid config");
});

const human = {
	runId: "run",
	revision: 2,
	status: "waitingHuman",
	step: { id: "help", name: "Help", human: true },
	attempt: 3,
	allowedOutcomes: ["approved", "rejected"],
	lastResult: { stepId: "build", attempt: 2, output: "Please approve" },
};

test("runs steps through CLI and shows the result", async () => {
	const commands: string[] = [];
	const messages: string[] = [];
	const statuses: (string | undefined)[] = [];
	let completed = false;
	await drive(
		"run",
		async (args) => {
			commands.push(args[0]);
			if (args[0] === "execute") {
				completed = true;
				return {};
			}
			return completed
				? {
						status: "succeeded",
						lastResult: { stepId: "build", attempt: 1, output: "Done" },
					}
				: {
						status: "running",
						step: { id: "build", name: "Build" },
						attempt: 1,
					};
		},
		ui({ setStatus: (_key, value) => statuses.push(value) }),
		(text) => messages.push(text),
	);
	expect(commands).toEqual(["next", "execute", "next", "next", "status"]);
	expect(messages.join("\n")).toContain("Baton run — RUNNING");
	expect(statuses).toContain("Baton · RUNNING · Build · attempt 1");
	expect(messages.join("\n")).toContain("Attempt: 1");
	expect(messages.join("\n")).toContain("Done");
});

test("shows token usage when a Baton run reaches its terminal state", async () => {
	const messages: string[] = [];
	let nextCalls = 0;
	await drive("run", async (args) => {
		if (args[0] === "next") {
			nextCalls++;
			return nextCalls === 1
				? { status: "running", step: { id: "build", name: "Build" }, attempt: 1 }
				: { status: "succeeded" };
		}
		if (args[0] === "execute") return {};
		if (args[0] === "status") {
			return {
				steps: [{
					stepId: "build", attempt: 1, status: "succeeded",
					usage: { inputTokens: 1200, outputTokens: 300, cacheReadTokens: 10, cacheWriteTokens: 0, estimatedCost: "0.02", currency: "USD", priceUnit: "perMillionTokens", coverage: "complete" },
				}],
			};
		}
		return {};
	}, ui(), (message) => messages.push(message));
	expect(messages.at(-1)).toContain("Tokens input/output/cache read/cache write: 1200 (complete) / 300 (complete) / 10 (complete) / 0 (complete)");
	expect(messages.at(-1)).toContain("Run cost: 0.02 USD");
});

test("polls fixed progress while execute is pending and stops after completion", async () => {
	const commands: string[] = [];
	const statuses: (string | undefined)[] = [];
	let done = false;
	await drive("run", async (args) => {
		commands.push(args[0]);
		if (args[0] === "execute") {
			await new Promise((resolve) => setTimeout(resolve, 1100));
			done = true;
			return {};
		}
		if (args[0] === "progress")
			return { activity: "Tool activity", step: "build", attempt: 1 };
		return done
			? { status: "succeeded", lastResult: { stepId: "build", attempt: 1, output: "Done" } }
			: { status: "running", step: { id: "build", name: "Build" }, attempt: 1 };
	}, ui({ setStatus: (_key, value) => statuses.push(value) }), () => {});
	expect(commands).toContain("progress");
	expect(statuses.some((status) => status?.endsWith("Baton · RUNNING · build · attempt 1 · Tool activity"))).toBe(true);
	const frames = statuses.filter((status) => status?.startsWith("⠋ ") || status?.startsWith("⠙ ") || status?.startsWith("⠹ ") || status?.startsWith("⠸ ") || status?.startsWith("⠼ ") || status?.startsWith("⠴ ") || status?.startsWith("⠦ ") || status?.startsWith("⠧ ") || status?.startsWith("⠇ ") || status?.startsWith("⠏ "));
	expect(new Set(frames.map((status) => status?.slice(0, 1))).size).toBeGreaterThan(1);
	expect(statuses.at(-1)).toBeUndefined();
});

test("poll failures do not replace execute results", async () => {
	const messages: string[] = [];
	let done = false;
	await drive("run", async (args) => {
		if (args[0] === "execute") {
			await new Promise((resolve) => setTimeout(resolve, 1100));
			done = true;
			return {};
		}
		if (args[0] === "progress") throw new Error("poll unavailable");
		return done
			? { status: "succeeded", lastResult: { stepId: "build", attempt: 1, output: "Done" } }
			: { status: "running", step: { id: "build", name: "Build" }, attempt: 1 };
	}, ui(), (message) => messages.push(message));
	expect(messages.join("\\n")).toContain("Done");
});

test("does not overlap polls and stops polling after execute completes", async () => {
	let done = false;
	let inFlight = 0;
	let maximumInFlight = 0;
	let progressCalls = 0;
	await drive("run", async (args) => {
		if (args[0] === "execute") {
			await new Promise((resolve) => setTimeout(resolve, 2500));
			done = true;
			return {};
		}
		if (args[0] === "progress") {
			progressCalls++;
			inFlight++;
			maximumInFlight = Math.max(maximumInFlight, inFlight);
			await new Promise((resolve) => setTimeout(resolve, 1300));
			inFlight--;
			return { activity: "Tool activity" };
		}
		return done
			? { status: "succeeded", lastResult: { stepId: "build", attempt: 1, output: "Done" } }
			: { status: "running", step: { id: "build", name: "Build" }, attempt: 1 };
	}, ui(), () => {});
	const completedPollCount = progressCalls;
	await new Promise((resolve) => setTimeout(resolve, 1100));
	expect(progressCalls).toBe(completedPollCount);
	expect(maximumInFlight).toBe(1);
});

test("polling does not hide a human pause returned after execute", async () => {
	const messages: string[] = [];
	let executed = false;
	await drive("run", async (args) => {
		if (args[0] === "execute") {
			await new Promise((resolve) => setTimeout(resolve, 1100));
			executed = true;
			return {};
		}
		if (args[0] === "progress") return { activity: "Runtime activity" };
		if (executed) return human;
		return { status: "running", step: { id: "build", name: "Build" }, attempt: 1 };
	}, ui(), (message) => messages.push(message));
	expect(messages.join("\\n")).toContain("Waiting for your decision");
});

test("clears polling when execute is interrupted", async () => {
	const statuses: (string | undefined)[] = [];
	let resolvePoll: (() => void) | undefined;
	await expect(drive("run", async (args) => {
		if (args[0] === "execute") {
			await new Promise((resolve) => setTimeout(resolve, 1100));
			throw new Error("execution interrupted");
		}
		if (args[0] === "progress")
			return new Promise((resolve) => { resolvePoll = () => resolve({ activity: "Runtime activity" }); });
		return { status: "running", step: { id: "build", name: "Build" }, attempt: 1 };
	}, ui({ setStatus: (_key, value) => statuses.push(value) }), () => {})).rejects.toThrow("execution interrupted");
	const callsAfterInterruption = statuses.length;
	resolvePoll?.();
	await new Promise((resolve) => setTimeout(resolve, 0));
	await new Promise((resolve) => setTimeout(resolve, 1100));
	expect(statuses.length).toBe(callsAfterInterruption);
	expect(statuses.at(-1)).toBeUndefined();
});

test("formats extension failures as Baton error feedback", async () => {
	let handler: (args: string, ctx: any) => Promise<void> = async () => {};
	const notices: string[] = [];
	const pi = {
		on: () => () => {},
		registerCommand: (_name: string, options: { handler: typeof handler }) => { handler = options.handler; },
		exec: async () => ({ code: 1, killed: false, truncated: false, stderr: "offline", stdout: "" }),
	} as unknown as ExtensionAPI;
	baton(pi);
	await handler("run-1", { hasUI: true, cwd: "/tmp", ui: { notify: (message: string) => notices.push(message), setStatus() {} } } as any);
	expect(notices[0]).toContain("Baton run-1 — ERROR");
	expect(notices[0]).toContain("offline");
});

test("makes a human handoff prominent with decisions and resume instructions", async () => {
	const messages: string[] = [];
	await drive("run-1", async () => human, ui(), (text) => messages.push(text));
	expect(messages[0]).toContain("Waiting for your decision");
	expect(messages[0]).toContain("approved, rejected");
	expect(messages[0]).toContain("resume run-1");
});

test("pages long human handoffs on demand without flooding the terminal", async () => {
	const messages: string[] = [];
	const choices: string[] = [];
	const output = `${"A".repeat(7000)}${"B".repeat(7000)}${"C".repeat(7000)}`;
	await drive("run-1", async () => ({ ...human, lastResult: { ...human.lastResult!, output } }), ui({
		select: async (prompt, options) => {
			choices.push(prompt);
			return prompt.includes("handoff") ? options[0] : undefined;
		},
	}), (text) => messages.push(text));
	expect(messages).toHaveLength(4);
	expect(messages[0]).toContain("resume run-1");
	expect(messages[0].indexOf("resume run-1")).toBeLessThan(messages[0].indexOf("Prior handoff:"));
	expect(messages.slice(1)).toEqual(["A".repeat(7000), "B".repeat(7000), "C".repeat(7000)]);
	expect(messages.every((message) => message.length <= 8000)).toBe(true);
	expect(choices.filter((prompt) => prompt.includes("handoff"))).toHaveLength(2);
});

test("does not submit a human decision when selection is cancelled", async () => {
	const commands: string[] = [];
	await drive(
		"run",
		async (args) => {
			commands.push(args[0]);
			return human;
		},
		ui(),
		() => {},
	);
	expect(commands).toEqual(["next"]);
});

test("does not submit a decision when confirmation is declined", async () => {
	const commands: string[] = [];
	await drive(
		"run",
		async (args) => {
			commands.push(args[0]);
			return human;
		},
		ui({
			select: async () => "approved",
			editor: async () => "Proceed",
			confirm: async () => false,
		}),
		() => {},
	);
	expect(commands).toEqual(["next"]);
});

test("submits only the explicitly confirmed human decision", async () => {
	let submitted: unknown;
	let waiting = true;
	const messages: string[] = [];
	await drive(
		"run",
		async (args) => {
			if (args[0] === "next") return waiting ? human : { status: "succeeded" };
			if (args[0] === "respond") {
				submitted = JSON.parse(
					await readFile(args[args.indexOf("--result") + 1], "utf8"),
				);
				waiting = false;
				return {};
			}
			throw new Error("unexpected command");
		},
		ui({
			select: async () => "approved",
			editor: async () => "Proceed",
			confirm: async () => true,
		}),
		(text) => messages.push(text),
	);
	expect(submitted).toEqual({
		revision: 2,
		stepId: "help",
		attempt: 3,
		outcome: "approved",
		response: "Proceed",
	});
	expect(messages.join("\n")).toContain("Please approve");
});

test("clears the status when a CLI command fails", async () => {
	const statuses: (string | undefined)[] = [];
	await expect(
		drive(
			"run",
			async () => {
				throw new Error("offline");
			},
			ui({
				setStatus: (_key, value) => {
					statuses.push(value);
				},
			}),
			() => {},
		),
	).rejects.toThrow("offline");
	expect(statuses).toEqual([undefined]);
});
