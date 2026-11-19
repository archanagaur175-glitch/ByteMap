// Minimal typing for the single Node API a test uses. Vitest runs in Node,
// but this project deliberately avoids @types/node (the brief's dependency
// list is closed), so only the one signature needed is declared here.
declare module "node:fs" {
  export function readFileSync(path: URL | string, encoding: "utf8"): string;
}
