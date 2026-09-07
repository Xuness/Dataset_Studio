import { cargo, finished } from "./cargo.mjs";
await finished(cargo(process.argv.slice(2)));
