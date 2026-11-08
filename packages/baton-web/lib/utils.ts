import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

export function cn(...inputs: ClassValue[]) {
	return twMerge(clsx(inputs));
}

export type WithoutChild<T> = Omit<T, "child">;
export type WithoutChildrenOrChild<T> = Omit<T, "child" | "children">;
export type WithElementRef<T, U extends HTMLElement = HTMLElement> = T & {
	ref?: U | null;
};
