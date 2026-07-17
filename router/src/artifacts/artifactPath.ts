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
  if (!isCanonicalArtifactRelativePath(artifactPath)) {
    throw new Error(
      `${indexPath} artifact path must be canonical and relative: ${artifactPath}`,
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

function isCanonicalArtifactRelativePath(path: string): boolean {
  const windowsDrive = /^[A-Za-z]:/.test(path);
  return (
    path.length > 0
    && !isAbsolute(path)
    && !windowsDrive
    && !path.includes("\\")
    && path.split("/").every((part) => part.length > 0 && part !== "." && part !== "..")
  );
}

function escapesRoot(path: string): boolean {
  return (
    path === "" ||
    path.startsWith("..") ||
    path.split(sep).includes("..") ||
    isAbsolute(path)
  );
}
