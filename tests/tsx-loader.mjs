// 仅 node:test 使用已安装的 TypeScript 转译真实组件；不参与生产构建。
import { registerHooks } from 'node:module'
import { existsSync, readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import ts from 'typescript'

registerHooks({
  resolve(specifier, context, nextResolve) {
    if (specifier.startsWith('@/')) {
      const base = new URL(`../src/${specifier.slice(2)}`, import.meta.url)
      const url = [base.href, `${base.href}.tsx`, `${base.href}.ts`].find(value => existsSync(fileURLToPath(value)))
      if (url) return { url, shortCircuit: true }
    }
    return nextResolve(specifier, context)
  },
  load(url, context, nextLoad) {
    if (url.endsWith('.tsx')) {
      const source = ts.transpileModule(readFileSync(new URL(url), 'utf8'), {
        compilerOptions: { jsx: ts.JsxEmit.ReactJSX, target: ts.ScriptTarget.ES2023, module: ts.ModuleKind.ESNext },
        fileName: fileURLToPath(url),
      }).outputText
      return { format: 'module', source, shortCircuit: true }
    }
    return nextLoad(url, context)
  },
})
