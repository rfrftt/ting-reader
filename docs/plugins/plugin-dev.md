# 插件开发指南

Ting Reader 支持通过插件扩展功能，包括元数据刮削和音频格式支持。您可以根据需求选择适合的开发方式。

- **JavaScript 刮削插件**: 最简单的开发方式，使用 JavaScript 编写，运行在轻量级运行时中。适合编写 HTTP 请求驱动的刮削逻辑。
- **WASM 刮削插件**: 使用 Rust 编写并编译为 WebAssembly。提供比 JS 更好的性能和类型安全，支持复杂的解析逻辑。
- **Native 格式插件**: 使用 Rust 编写并编译为动态链接库。拥有完全的系统权限，适合处理复杂的音频格式解码和加密文件。

## 插件配置文件 (plugin.json)

每个插件都需要在根目录提供一个 `plugin.json` 配置文件，用于向 Ting Reader 声明插件的基本信息、入口、权限和配置项。

### 配置文件示例

以下是一个典型的 `plugin.json` 示例：

```json
{
  "id": "my-scraper-js",
  "name": "My JS Scraper",
  "version": "1.0.0",
  "plugin_type": "scraper",
  "author": "Your Name",
  "description": "A simple JS scraper example",
  "runtime": "javascript",
  "entry_point": "plugin.js",
  "permissions": [
    { "type": "network_access", "value": "*.example.com" },
    { "type": "network_access", "value": "api.another-site.com" }
  ]
}
```

### 字段说明

以下是所有支持的字段及其说明：

- **`id`** *(必填, string)*: 插件的全局唯一标识符，通常使用小写字母和连字符（如 `"douban-scraper-js"`）。
- **`name`** *(必填, string)*: 插件的显示名称，在管理后台中展示。
- **`version`** *(必填, string)*: 插件的版本号，遵循语义化版本规范（如 `"1.0.0"`）。
- **`plugin_type`** *(必填, string)*: 插件的类型。支持的值包括 `"scraper"`（刮削器插件）、`"format"`（格式支持插件）和 `"utility"`（工具类插件）。部分插件也兼容使用 `type` 字段。
- **`author`** *(必填, string)*: 插件作者名称。
- **`description`** *(必填, string)*: 插件的功能简要描述。
- **`description_en`** *(可选, string)*: 插件的英文描述。
- **`entry_point`** *(必填, string)*: 插件的执行入口文件名称（如 `"plugin.js"`, `"xm_format.dll"`, `"ypshuo_scraper.wasm"`）。
- **`runtime`** *(可选, string)*: 插件的运行环境。对于 JavaScript 插件为 `"javascript"`，WASM 插件为 `"wasm"`。Native 插件通常不需要此字段。
- **`homepage`** *(可选, string)*: 插件的主页或代码仓库 URL。
- **`license`** *(可选, string)*: 插件的开源许可证（如 `"MIT"`）。
- **`dependencies`** *(可选, array)*: 插件所依赖的其他插件 `id` 列表（如 `["ffmpeg-utils"]`）。
- **`npm_dependencies`** *(可选, array)*: （仅限 JavaScript 插件）需要预先加载的 NPM 依赖包列表。例如 `[{"name": "axios", "version": "^1.6.0"}]`。
- **`permissions`** *(可选, array)*: 插件运行所需的系统权限列表。为了安全，需要显式声明权限。
  - `type`: 权限类型，包括 `"network_access"`（网络访问）、`"file_read"`（读取文件）、`"file_write"`（写入文件）。
  - `value`: 权限对应的目标，如域名 `"*.douban.com"` 或相对路径 `"./data/audio"`。
- **`supported_extensions`** *(可选, array)*: （仅限格式插件）支持解析或处理的音频文件扩展名列表（如 `["xm"]`, `["m4a", "flac"]`）。
- **`config_schema`** *(可选, object)*: 插件的自定义配置项结构，使用类似 JSON Schema 的格式定义。在系统后台会根据此字段自动生成插件设置表单。支持定义 `type`, `title`, `description`, `default` 等属性。

