import { describe, expect, test } from "bun:test";
import { parseRunActivity } from "./run-activity";

describe("parseRunActivity", () => {
	test("extracts tool calls, results and assistant text in order", () => {
		const content = JSON.stringify({
			messages: [
				{ role: "user", content: [{ type: "text", text: "task" }] },
				{
					role: "assistant",
					content: [
						{ type: "reasoning", text: "private" },
						{
							type: "toolcall",
							function: { name: "read", arguments: { path: "a" } },
						},
					],
				},
				{
					role: "user",
					content: [
						{
							type: "toolresult",
							name: "read",
							content: [{ type: "text", text: "file" }],
						},
					],
				},
				{ role: "assistant", content: [{ type: "text", text: "done" }] },
			],
		});
		expect(parseRunActivity(content).map((item) => item.label)).toEqual([
			"Tool call: read",
			"Tool result: read",
			"Agent response",
		]);
	});
	test("presents redacted model and tool lifecycle metadata", () => {
		expect(
			parseRunActivity(
				JSON.stringify({
					kind: "modelRequest",
					messageRoles: ["user"],
					toolNames: ["read"],
				}),
			),
		).toEqual([
			{
				label: "Model request",
				content: "Messages: user; configured tools: read",
			},
		]);
		expect(
			parseRunActivity(
				JSON.stringify({ kind: "modelResponse", status: "completed" }),
			)[0].label,
		).toBe("Model response: completed");
		expect(
			parseRunActivity(JSON.stringify({ kind: "toolCall", tool: "read" }))[0],
		).toEqual({
			label: "Tool call: read",
			content: "Arguments are omitted from the persisted trace.",
		});
		expect(
			parseRunActivity(
				JSON.stringify({ kind: "toolResult", tool: "read", status: "success" }),
			)[0].label,
		).toBe("Tool result: read · success");
		expect(
			parseRunActivity(
				JSON.stringify({ kind: "traceTruncated", content: "capture limit" }),
			)[0].label,
		).toBe("Trace capture truncated");
	});
	test("ignores invalid and unknown transcripts", () => {
		expect(parseRunActivity("not json")).toEqual([]);
		expect(parseRunActivity("{}")).toEqual([]);
	});
});
