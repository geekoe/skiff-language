import { readFile, realpath } from "node:fs/promises";
import { isAbsolute, relative, resolve, sep } from "node:path";

export async function readJsonAtArtifactPath(
  root: string,
  artifactPath: string,
  indexPath: string,
): Promise<unknown> {
  const path = await resolveArtifactPath(root, artifactPath, indexPath);
  const text = await readFile(path, "utf8");
  return JSON.parse(text);
}

export async function resolveArtifactPath(
  root: string,
  artifactPath: string,
  indexPath: string,
): Promise<string> {
  if (artifactPath.length === 0 || isAbsolute(artifactPath)) {
    throw new Error(
      `${indexPath} artifact path must be relative: ${artifactPath}`,
    );
  }
  const path = resolve(root, artifactPath);
  const relativePath = relative(root, path);
  if (escapesRoot(relativePath)) {
    throw new Error(
      `${indexPath} artifact path escapes artifact root: ${artifactPath}`,
    );
  }
  const [realRoot, realPath] = await Promise.all([
    realpath(root),
    realpath(path).catch((error: unknown) => {
      throw new Error(
        `${indexPath} failed to resolve artifact path ${artifactPath}`,
        {
          cause: error,
        },
      );
    }),
  ]);
  if (escapesRoot(relative(realRoot, realPath))) {
    throw new Error(
      `${indexPath} artifact path escapes artifact root: ${artifactPath}`,
    );
  }
  return realPath;
}

function escapesRoot(path: string): boolean {
  return (
    path === "" ||
    path.startsWith("..") ||
    path.split(sep).includes("..") ||
    isAbsolute(path)
  );
}
