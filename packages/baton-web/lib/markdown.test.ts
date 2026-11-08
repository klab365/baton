import { expect, test } from "bun:test";
import { renderMarkdown } from "./markdown";

test("renders Markdown and preserves plain text line breaks", () => {
	const html = renderMarkdown("**Bold**\nsecond line");
	expect(html).toContain("<strong>Bold</strong>");
	expect(html).toContain("<br />");
});

test("renders headings, lists, quotes and code", () => {
	const html = renderMarkdown(
		"## Heading\n\n- one\n- two\n\n> quote\n\n```js\nconst x = 1;\n```",
	);
	expect(html).toContain("<h2>Heading</h2>");
	expect(html).toContain("<li>one</li>");
	expect(html).toContain("<blockquote>");
	expect(html).toContain("<pre><code");
});

test("removes unsafe HTML, links and remote images", () => {
	const html = renderMarkdown(
		"<script>alert(1)</script>\n\n[bad](javascript:alert(1)) ![tracking](https://example.com/a) <img src=x onerror=alert(1)>",
	);
	expect(html).not.toContain("<script");
	expect(html).not.toContain("javascript:");
	expect(html).not.toContain("<img");
	expect(html).not.toContain("onerror");
});
