import {stdin, stdout} from "node:process";
import {prepareSource, projectPaths, runCommand} from "./project.mjs";

try {
    const chunks = [];
    for await (const chunk of stdin) chunks.push(chunk);
    const request = JSON.parse(Buffer.concat(chunks).toString("utf8"));
    const {operation, command, project, session} = request;
    let result, bytes;
    switch (operation) {
        case "source":
            ({source: result, bytes} = await prepareSource(command));
            break;
        case "process":
            ({bytes, ...result} = await runCommand(projectPaths(project), session, command));
            break;
        default:
            throw new Error("Unknown worker operation");
    }
    stdout.write(JSON.stringify({result}) + "\n");
    if (bytes) stdout.write(bytes);
} catch (error) {
    stdout.write(JSON.stringify({error: error.message}) + "\n");
}
