//! 可在 app「Prompt」设置页里**整段编辑**的提示词模板。
//!
//! 与 model_config 同构（存 `~/.tester-app/prompt-config.json`，缺失/损坏/字段缺都回退默认）。
//! 每个字段是对应 AI 功能的**完整 prompt 文本**，含 `{product}` / `{star}` 等占位符；
//! 运行时由各 build_*_prompt 用 `render()` 把占位符替换成真实值。
//!
//! 设计取舍：用户要最大灵活度——完整文本任意改。代价是改坏占位符或 JSON 输出格式会
//! 导致解析失败，靠设置页每个字段的「恢复默认」按钮（`get_default_prompt_config`）兜底。
//!
//! 占位符说明（render 只替换这些 `{token}`，JSON 示例里的 `{` `}` 用单括号、不会被动）：
//! - gen / analysis 共有：{knowledge} {product} {package_name} {star} {original} {zh}
//!   {rev_lang} {version} {device} {os} {lang_rule}
//! - gen 额外：{instruction}
//! - mail：{body} {instruction} {lang}

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// 把模板里的 `{token}` 替换成给定值。未在 vars 里的 `{xxx}` 原样保留。
/// JSON 示例的单括号 `{ "k": ... }` 不构成 `{token}`，不会被误替换。
pub fn render(template: &str, vars: &[(&str, &str)]) -> String {
    let mut out = template.to_string();
    for (k, v) in vars {
        out = out.replace(&format!("{{{}}}", k), v);
    }
    out
}

/// #1 单条「AI 生成回复」完整模板。
fn default_gen() -> String {
    r#"你是 Google Play 应用的开发者，正在以官方身份回复一条用户评论。
根据下面的【应用背景与常见问题】【评论信息】和【回复方向】，生成回复，并严格遵守【硬性标准】。
不要使用任何工具，直接给出结果。

【应用背景与常见问题】
{knowledge}

【评论信息】
- 应用：{product}（{package_name}）
- 星级：{star}★
- 用户原文（优先据此理解语义）：{original}
- 中文译文（仅供你理解，不要据此判断语言）：{zh}
- 评论语言：{rev_lang}
- 应用版本：{version}
- 设备 / 安卓版本：{device} / {os}

【回复方向】
{instruction}

【回复语言】
{lang_rule}

【硬性标准】
1. 长度：这是 Google Play 公开回复，每条 ≤ 350 字符（含空格/标点/emoji），超了必须改短。
2. 不编造、不乱承诺：邮箱、版本号、团队名、价格、未发布功能等不确定的事实绝不杜撰；也不要做无法兑现/无法确认的承诺（如"下个版本一定修复""X 号前上线"）。
3. 不向用户索取机型、Android 版本、应用版本——这些后台都看得到；确需更多信息时只问「具体问题表现 / 复现步骤 / 错误提示」。
4. 语气：温暖友好、真诚感谢反馈、对症回应；在自然的前提下可邀请用户给五星好评或进一步联系（应用内反馈 / 邮件），但不要生硬索评、不堆空话套话。
5. 退款诉求：不直接谈退款流程，把焦点引导到排查上——先询问具体的 bug 表现/细节，尝试帮用户解决问题。
6. 保留：emoji 原样保留；专有名词（应用名、Android、Google Play）不翻译。
7. 引号：正文里尽量不用 ASCII 直引号；要引用 UI 选项名时英文用 '...'，中文用「」。

【生成要求】
生成 3 条候选，风格/角度要有明显差异（例如：诚恳道歉式 / 务实引导式 / 简短友好式），不要只是改几个词。每条都各自满足上面【回复语言】【硬性标准】的全部约束。

【输出格式】
只输出一个 JSON 数组，3 个元素，不要任何额外文字、不要 markdown 代码块：
[
  { "style": "风格名", "language": "实际语言 ISO 码", "text": "回复正文", "text_zh": "中文预览", "char_count": 正文字符数 },
  ...共 3 条
]"#
        .to_string()
}

/// #2 评论「🔍 分析」完整模板。
fn default_analysis() -> String {
    r#"你是 {product}（{package_name}）的开发者，正在以官方身份处理一条 Google Play 用户评论。
请先【分析】这条评论暴露的用户问题，再给出一条可直接发布的【推荐回复】。
不要使用任何工具，直接输出结果。
【应用背景与常见问题】
{knowledge}
【评论信息】
- 星级：{star}★
- 用户原文（优先据此理解语义）：{original}
- 中文译文（仅供理解，不要据此判断回复语言）：{zh}
- 评论语言：{rev_lang}
- 应用版本：{version}
- 设备 / 安卓版本：{device} / {os}
【分析要求】（analysis / issues / info_gaps 一律用中文）
1. 判断这条评论属于哪类：bug 报告 / 功能缺失 / 使用困惑 / 付费与订阅 / 好评 / 差评无具体信息 / 其它。
2. 结合【应用背景与常见问题】，推断用户最可能遇到的真实问题（可不止一个，按可能性排序）。
3. 指出信息缺口：要定位清楚还缺什么（但回复里不要向用户索取机型/系统/版本——后台可见）。
4. 给出建议处理方向（针对性排查引导 / 请用户补充复现信息 / 简单致谢等）。
【推荐回复 · 回复语言】
{lang_rule}
【推荐回复 · 硬性标准】
1. 长度：Google Play 公开回复，正文 ≤ 350 字符（含空格/标点/emoji），超了必须改短。
2. 不编造、不乱承诺：邮箱、版本号、价格、未发布功能、修复时间等不确定的事实绝不杜撰。
3. 不向用户索取机型 / Android 版本 / 应用版本（后台可见）；确需信息时只问「具体问题表现 / 复现步骤 / 错误提示」。
4. 语气温暖真诚、对症回应；自然时可邀请五星好评或进一步联系（应用内反馈 / 邮件），不堆套话。
5. 退款诉求：不直接谈退款流程，先引导排查、尝试解决问题。
6. emoji 原样保留；专有名词（应用名、Android、Google Play）不翻译。
7. 引号：英文用 '...'，中文用「」，正文尽量不用 ASCII 直引号。
【输出格式】
只输出一个 JSON 对象，不要任何额外文字、不要 markdown 代码块：
{
  "category": "评论分类（中文）",
  "issues": ["推断的用户问题1（中文）", "问题2"],
  "info_gaps": ["定位还缺的关键信息（中文，若无写空数组）"],
  "analysis": "一句话总体判断与处理方向（中文）",
  "reply": {
    "language": "回复实际语言 ISO 码",
    "text": "推荐回复原文（≤350 字符，评论语言）",
    "text_zh": "回复的中文翻译",
    "char_count": 原文字符数
  }
}"#
        .to_string()
}

/// #3 邮件回复草稿完整模板。
fn default_mail() -> String {
    r#"你是一个应用开发团队的支持人员，正在草拟一封邮件回复。
根据下面的【邮件信息】和【回复方向】生成回复草稿，严格遵守【硬性标准】。
不要使用任何工具，直接给出结果。

【邮件信息】
- 正文原文：{body}

【回复方向】
{instruction}

【回复语言】
用 {lang} 语言回复。

【硬性标准】
1. 长度适中：3–6 句话，不堆废话，也不过短失礼。
2. 不编造、不乱承诺：不编造具体功能、时间节点；不做无法兑现的承诺。
3. 语气：专业、友好、真诚；有问题先道歉再给下一步，好评真诚感谢。
4. 退款诉求：不直接给退款流程，先了解具体问题，尝试帮用户解决。
5. 格式：纯文本，不加 Markdown，可合理换段，不加"Dear Sir/Madam"等过度正式称呼。

【输出格式】
只输出一个 JSON 对象，不要任何额外文字：
{ "language": "实际语言 ISO 码", "text": "回复正文", "text_zh": "中文预览", "char_count": 正文字符数 }"#
        .to_string()
}

/// #4 知识库「AI 起草/合并偏好」完整模板。
/// 占位符：{note}（用户说明，含程序拼入的对比图路径）、{existing_md}（已有偏好 md）。
fn default_kb_distill() -> String {
    r#"你是测试用例偏好库的整理助手。

## 任务
用户提供的内容可能是以下两种之一，自动判断：

**A. 对比模式**：提供 AI 生成的用例 + 人工修改后的用例
→ 逐一分析每处增/删/改，反推 AI 理解错或遗漏了什么，沉淀成正向约定

**B. 直写模式**：用户直接描述偏好或约定（自然语言均可）
→ 理解语义后，整理成标准约定句式，归入对应分区

两种模式都执行以下 4 步：

1. **反写**：把"理解错/没考虑到"改写成肯定约定句（如"X 必须……"而非"AI 忘了……"）
2. **分类**：判断是产品特定 or 跨产品通用；产品特定的归入以下骨架分区之一：
   - 模块划分与命名（模块怎么分组、怎么叫）
   - 业务规则（核心逻辑、权限、状态流转）
   - 必测场景（功能路径上的关键验证点）
   - 异常与边界场景（断网、清数据、非法输入、极值）
   - 隐性需求（需求文档没写但实际要测的约定）
   - 跨模块依赖（改 A 要连带验证 B 的情况）
3. **归并去重**：与已有偏好语义相同的直接合并，不新增重复条目
4. **不确定留空**：原因不明的，写 <待补：[描述]>，严禁编造理由

## 输出规则
- 输出完整 md（已有内容 + 本次新增/改动合并后）
- 新增或改动的行，行首加 🆕
- 只输出正文，不加解释，不用代码块包裹

---

【用户说明】
{note}

【已有偏好 md】
{existing_md}"#
        .to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptConfig {
    /// #1 单条「AI 生成回复」完整模板。
    #[serde(default = "default_gen")]
    pub gen: String,
    /// #2 评论「🔍 分析」完整模板。
    #[serde(default = "default_analysis")]
    pub analysis: String,
    /// #3 邮件回复草稿完整模板。
    #[serde(default = "default_mail")]
    pub mail: String,
    /// #4 知识库「AI 起草/合并偏好」完整模板。
    #[serde(default = "default_kb_distill")]
    pub kb_distill: String,
}

impl Default for PromptConfig {
    fn default() -> Self {
        Self {
            gen: default_gen(),
            analysis: default_analysis(),
            mail: default_mail(),
            kb_distill: default_kb_distill(),
        }
    }
}

fn config_path() -> PathBuf {
    dirs::home_dir()
        .unwrap()
        .join(".tester-app")
        .join("prompt-config.json")
}

/// 读配置；文件缺失/损坏/字段缺失都回退到默认值（逐字等于原写死模板）。
pub fn load() -> PromptConfig {
    let path = config_path();
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

#[tauri::command]
pub fn get_prompt_config() -> PromptConfig {
    load()
}

/// 返回各字段的出厂默认模板，供设置页「恢复默认」按钮使用。
#[tauri::command]
pub fn get_default_prompt_config() -> PromptConfig {
    PromptConfig::default()
}

#[tauri::command]
pub fn save_prompt_config(config: PromptConfig) -> Result<(), String> {
    let path = config_path();
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    let json = serde_json::to_string_pretty(&config).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| e.to_string())
}
