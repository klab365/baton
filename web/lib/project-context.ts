import { basename, resolve } from "node:path";

const projectDirectory = process.env.BATON_PROJECT_DIR ?? process.cwd();

export const projectContext = {
	directory: resolve(projectDirectory),
	name: basename(resolve(projectDirectory)),
};
