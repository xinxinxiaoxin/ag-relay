import test from 'node:test'
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { desktopWindow } from '../src/lib/window.ts'

const read = path => readFileSync(new URL(path, import.meta.url))
const config = JSON.parse(read('../src-tauri/tauri.conf.json'))
const capability = JSON.parse(read('../src-tauri/capabilities/default.json'))

test('普通浏览器不提供原生窗口对象', () => {
  assert.equal(desktopWindow(), null)
})

test('桌面窗口使用当前窗口对象而不是新建窗口', () => {
  const previous = globalThis.__TAURI__
  const nativeWindow = { close() {}, minimize() {}, toggleMaximize() {} }
  globalThis.__TAURI__ = { window: { getCurrentWindow: () => nativeWindow } }
  try {
    assert.equal(desktopWindow(), nativeWindow)
  } finally {
    if (previous === undefined) delete globalThis.__TAURI__
    else globalThis.__TAURI__ = previous
  }
})

test('一体式浅色窗口保留阴影和最小尺寸', () => {
  const main = config.app.windows.find(window => window.label === 'main')
  assert.equal(main.decorations, false)
  assert.equal(main.shadow, true)
  assert.equal(main.theme, 'Light')
  assert.equal(main.minWidth, 960)
  assert.equal(main.minHeight, 640)
  assert.notEqual(main.resizable, false)
})

test('窗口权限只允许关闭请求而非直接销毁', () => {
  for (const action of ['start-dragging', 'minimize', 'toggle-maximize', 'close']) {
    assert.ok(capability.permissions.includes(`core:window:allow-${action}`))
  }
  assert.ok(!capability.permissions.includes('core:window:allow-destroy'))
  assert.deepEqual(capability.windows, ['main'])
})

test('ICO 包含从托盘级到桌面级的独立尺寸，且每个尺寸是有效 PNG', () => {
  const ico = read('../src-tauri/icons/icon.ico')
  assert.equal(ico.readUInt16LE(0), 0)
  assert.equal(ico.readUInt16LE(2), 1)
  const sizes = [16, 20, 24, 32, 48, 64, 96, 128, 256]
  assert.equal(ico.readUInt16LE(4), sizes.length)
  let expectedOffset = 6 + sizes.length * 16
  for (const [index, size] of sizes.entries()) {
    const entry = 6 + index * 16
    assert.equal(ico[entry] || 256, size)
    assert.equal(ico[entry + 1] || 256, size)
    const length = ico.readUInt32LE(entry + 8)
    const offset = ico.readUInt32LE(entry + 12)
    assert.equal(offset, expectedOffset)
    const png = ico.subarray(offset, offset + length)
    assert.equal(png.subarray(0, 8).toString('hex'), '89504e470d0a1a0a')
    assert.equal(png.readUInt32BE(16), size)
    assert.equal(png.readUInt32BE(20), size)
    assert.equal(png[25], 6)
    expectedOffset += length
  }
  assert.equal(expectedOffset, ico.length)
})

test('托盘使用独立 AG 资源，不复用人脸小图标', () => {
  const tray = read('../src-tauri/icons/tray.png')
  const face = read('../src-tauri/icons/32x32.png')
  assert.equal(tray.readUInt32BE(16), 32)
  assert.equal(tray.readUInt32BE(20), 32)
  assert.equal(tray[25], 6)
  assert.notDeepEqual(tray, face)
  assert.match(read('../src-tauri/src/tray.rs').toString(), /include_image!\("icons\/tray.png"\)/)
})
