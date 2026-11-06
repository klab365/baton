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
	test("ignores invalid and unknown transcripts", () => {
		expect(parseRunActivity("not json")).toEqual([]);
		expect(parseRunActivity("{}")).toEqual([]);
	});
});
