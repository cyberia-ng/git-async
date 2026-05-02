export class DirectoryWrapper {
  constructor(inner) {
    this.inner = inner;
  }

  async openSubdir(name) {
    return new DirectoryWrapper(await this.inner.openSubdir(name));
  }

  listDir() {
    return this.inner.listDir();
  }

  async openFile(name) {
    return new FileWrapper(await this.inner.openFile(name));
  }
}

export class FileWrapper {
  constructor(inner) {
    this.inner = inner;
  }

  readAll() {
    return this.inner.readAll();
  }

  readSegment(offset, length) {
    return this.inner.readSegment(offset, length);
  }
}

export class FSFile {
  constructor(file) {
    this.file = file;
  }

  async readAll() {
    const ab = await this.file.arrayBuffer();
    return new Uint8Array(ab);
  }

  async readSegment(offset, length) {
    const sliced = this.file.slice(offset, offset + length);
    const ab = await sliced.arrayBuffer();
    return new Uint8Array(ab);
  }
}

export class FSDirectory {
  constructor(handle) {
    this.handle = handle;
  }

  async openSubdir(name) {
    const handle = await this.handle.getDirectoryHandle(name);
    return new FSDirectory(handle);
  }

  async listDir() {
    const entries = await collect(this.handle.entries());
    const directories = entries
      .filter(([, handle]) => handle.kind === "directory")
      .map(([name]) => name);
    const files = entries.filter(([, handle]) => handle.kind === "file").map(([name]) => name);
    return [directories, files];
  }

  async openFile(name) {
    const handle = await this.handle.getFileHandle(name);
    const file = await handle.getFile();
    return new FSFile(file);
  }
}

export function entriesDirectoryFromFileList(fileList) {
  const first = fileList[0];
  if (first === undefined) {
    throw new Error("empty directory tree");
  }
  const rootName = first.webkitRelativePath.split("/")[0];
  const filesArray = Array.from(fileList);
  return new EntriesDirectory(filesArray, rootName + "/");
}

export class EntriesDirectory {
  constructor(files, subPath) {
    this.files = files;
    this.subPath = subPath;
  }

  async openSubdir(name) {
    const targetPath = this.subPath + name + "/";
    return new EntriesDirectory(
      this.files.filter((file) => file.webkitRelativePath.startsWith(targetPath)),
      targetPath,
    );
  }

  async listDir() {
    const subFiles = this.files.filter((file) => file.webkitRelativePath.startsWith(this.subPath));
    const directories = new Set();
    const files = [];
    for (const subFile of subFiles) {
      const relPath = subFile.webkitRelativePath.slice(this.subPath.length);
      const [firstComponent, ...rest] = relPath.split("/");
      if (firstComponent === undefined) {
        continue;
      }
      if (rest.length !== 0) {
        directories.add(firstComponent);
      } else {
        files.push(firstComponent);
      }
    }
    return [Array.from(directories), files];
  }

  async openFile(name) {
    const targetPath = this.subPath + name;
    const file = this.files.find((file) => file.webkitRelativePath === targetPath);
    if (file === undefined) {
      throw "file not found";
    }
    return new FSFile(file);
  }
}

async function collect(iter) {
  const out = [];
  for await (const item of iter) {
    out.push(item);
  }
  return out;
}
