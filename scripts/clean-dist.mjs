import { lstatSync, readdirSync, realpathSync, rmdirSync, unlinkSync } from 'node:fs'
import { dirname, isAbsolute, relative, resolve, sep } from 'node:path'
import { fileURLToPath } from 'node:url'

function abort(message) {
  throw new Error(`Refusing to clean dist: ${message}`)
}

function pathsMatch(left, right) {
  if (process.platform === 'win32') {
    return left.toLowerCase() === right.toLowerCase()
  }

  return left === right
}

function isWithinDirectory(directory, path) {
  const pathRelativeToDirectory = relative(directory, path)
  return (
    pathRelativeToDirectory === '' ||
    (!isAbsolute(pathRelativeToDirectory) &&
      pathRelativeToDirectory !== '..' &&
      !pathRelativeToDirectory.startsWith(`..${sep}`))
  )
}

function validateTree(path, realDistPath) {
  const stats = lstatSync(path)
  if (stats.isSymbolicLink()) {
    abort(`encountered a symbolic link or junction: ${path}`)
  }

  if (stats.isFile()) {
    return
  }

  if (!stats.isDirectory()) {
    abort(`encountered an unsupported filesystem entry: ${path}`)
  }

  const realPath = realpathSync.native(path)
  if (!isWithinDirectory(realDistPath, realPath)) {
    abort(`directory resolves outside dist: ${path}`)
  }

  for (const entryName of readdirSync(path)) {
    const entryPath = resolve(path, entryName)
    if (!isWithinDirectory(distPath, entryPath)) {
      abort(`entry resolves outside dist: ${entryPath}`)
    }

    validateTree(entryPath, realDistPath)
  }
}

function removeValidatedDirectory(path) {
  const stats = lstatSync(path)
  if (stats.isSymbolicLink() || !stats.isDirectory()) {
    abort(`filesystem entry changed while deleting: ${path}`)
  }

  for (const entryName of readdirSync(path)) {
    const entryPath = resolve(path, entryName)
    if (!isWithinDirectory(distPath, entryPath)) {
      abort(`entry resolves outside dist while deleting: ${entryPath}`)
    }

    const entryStats = lstatSync(entryPath)
    if (entryStats.isSymbolicLink()) {
      abort(`encountered a symbolic link or junction while deleting: ${entryPath}`)
    }

    if (entryStats.isDirectory()) {
      removeValidatedDirectory(entryPath)
    } else if (entryStats.isFile()) {
      unlinkSync(entryPath)
    } else {
      abort(`encountered an unsupported filesystem entry while deleting: ${entryPath}`)
    }
  }

  rmdirSync(path)
}

if (process.argv.length !== 2) {
  abort('path arguments are not supported')
}

const scriptPath = fileURLToPath(import.meta.url)
const scriptsDirectory = dirname(scriptPath)
const repoRoot = resolve(scriptsDirectory, '..')
const distPath = resolve(repoRoot, 'dist')

if (!pathsMatch(scriptsDirectory, resolve(repoRoot, 'scripts'))) {
  abort('the script must be located in the repository scripts directory')
}

const distRelativePath = relative(repoRoot, distPath)
if (
  distRelativePath !== 'dist' ||
  isAbsolute(distRelativePath) ||
  distRelativePath.startsWith(`..${sep}`)
) {
  abort('resolved target is not the repository dist directory')
}

let distStats
try {
  distStats = lstatSync(distPath)
} catch (error) {
  if (error && typeof error === 'object' && error.code === 'ENOENT') {
    console.log(`dist is already absent: ${distPath}`)
    process.exit(0)
  }

  throw error
}

if (distStats.isSymbolicLink()) {
  abort('dist is a symbolic link or junction')
}
if (!distStats.isDirectory()) {
  abort('dist is not a directory')
}

const realRepoRoot = realpathSync.native(repoRoot)
const expectedRealDistPath = resolve(realRepoRoot, 'dist')
const realDistPath = realpathSync.native(distPath)
if (!pathsMatch(realDistPath, expectedRealDistPath)) {
  abort('dist resolves outside the repository root')
}

const finalStats = lstatSync(distPath)
if (finalStats.isSymbolicLink() || !finalStats.isDirectory()) {
  abort('dist changed while it was being validated')
}

validateTree(distPath, expectedRealDistPath)
removeValidatedDirectory(distPath)

try {
  lstatSync(distPath)
  abort('dist still exists after deletion')
} catch (error) {
  if (error && typeof error === 'object' && error.code === 'ENOENT') {
    console.log(`Removed dist: ${distPath}`)
    process.exit(0)
  }

  throw error
}
