// model JSON -> ProseMirror doc -> model JSON (run after a Vite SSR build).
import { readFileSync, writeFileSync } from "node:fs";
import { docToModel, modelToDoc } from "../src/convert";

const [input, output] = process.argv.slice(2);
const model = JSON.parse(readFileSync(input, "utf8"));
const doc = modelToDoc(model);
doc.check();
writeFileSync(output, JSON.stringify(docToModel(doc)));
