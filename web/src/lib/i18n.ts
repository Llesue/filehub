import { writable } from 'svelte/store';

export type Lang = 'zh-CN' | 'en';

export const language = writable<Lang>('zh-CN');

const en: Record<string, string> = {
  存储目录: 'Storage directories',
  暂无目录: 'No locations',
  分享: 'Shares',
  分享管理: 'Share management',
  管理: 'Admin',
  设置: 'Settings',
  尚未接入: 'Not connected',
  仅管理员可用: 'Admin only',
  用户配额: 'Quota',
  物理磁盘: 'Disk',
  不限制: 'Unlimited',
  管理员: 'Admin',
  用户: 'User',
  退出登录: 'Sign out',
  关闭菜单: 'Close menu',
  打开菜单: 'Open menu',
  上传: 'Upload',
  '上传中…': 'Uploading…',
  上传文件: 'Upload file',
  当前目录只读: 'Read-only',
  搜索文件或目录: 'Search files or folders',
  视图: 'View',
  列表视图: 'List view',
  缩略图视图: 'Thumbnail view',
  平铺视图: 'Tile view',
  网格视图: 'Grid view',
  新建目录: 'New folder',
  上传目录: 'Upload folder',
  终端: 'Terminal',
  排序: 'Sort',
  请先选择存储目录: 'Choose a storage directory first',
  复制到: 'Copy to',
  移动到: 'Move to',
  重命名: 'Rename',
  取消: 'Cancel',
  切换到浅色模式: 'Switch to light mode',
  切换到深色模式: 'Switch to dark mode',
  文件: 'Files',
  搜索: 'Search',
  清除: 'Clear',
  没有找到匹配的文件: 'No matching files',
  '加载中…': 'Loading…',
  重试: 'Retry',
  已选: 'Selected',
  项: 'items',
  '下载 ZIP': 'Download ZIP',
  删除: 'Delete',
  这个目录是空的: 'This folder is empty',
  '上传文件或创建目录后，它们会显示在这里。': 'Upload files or create folders to see them here.',
  名称: 'Name',
  大小: 'Size',
  修改时间: 'Modified',
  链接: 'Link',
  目录: 'Folder',
  编辑: 'Edit',
  预览: 'Preview',
  关闭编辑器: 'Close editor',
  '保存中…': 'Saving…',
  保存: 'Save',
  首次设置: 'First-time setup',
  登录: 'Sign in',
  '创建第一个管理员账号，完成后即可开始使用。': 'Create the first admin account to get started.',
  '使用你的账号访问 FileHub。': 'Sign in to FileHub with your account.',
  设置令牌: 'Setup token',
  '启动日志中的 setup token': 'Setup token from the startup log',
  用户名: 'Username',
  密码: 'Password',
  确认密码: 'Confirm password',
  '处理中…': 'Processing…',
  创建管理员: 'Create admin',
  '设置令牌见服务启动日志，或由 FILEHUB_SETUP_TOKEN 指定。':
    'The setup token is printed in the startup log, or set via FILEHUB_SETUP_TOKEN.',
  两次输入的密码不一致: 'Passwords do not match',
  请求失败请稍后重试: 'Request failed, try again',
  无法打开分享: 'Unable to open share',
  密码错误: 'Incorrect password',
  下载失败: 'Download failed',
  'FileHub 分享': 'FileHub Share',
  该分享需要密码: 'This share requires a password.',
  输入分享密码: 'Enter share password',
  解锁: 'Unlock',
  分享路径: 'Share path',
  下载文件: 'Download file',
  主导航: 'Main navigation',
  文件导航: 'File navigation',
  存储状态: 'Storage status',
  文件内容: 'File content',
  关闭管理面板: 'Close admin panel',
  管理分类: 'Admin sections',
  新建用户: 'New user',
  配额字节空不限制: 'Quota bytes, empty = unlimited',
  创建: 'Create',
  用户列表: 'Users',
  已禁用: 'Disabled',
  正常: 'Active',
  保存配额: 'Save quota',
  启用: 'Enable',
  禁用: 'Disable',
  授权根: 'Roots',
  绝对路径: 'Absolute path',
  显示名: 'Display name',
  只读: 'Read-only',
  读写: 'Read/write',
  添加: 'Add',
  开关: 'Toggles',
  已开启: 'On',
  已关闭: 'Off',
  清除覆盖: 'Clear override',
  全部生效配置: 'Effective settings',
  登记可信软链接: 'Register trusted link',
  链接绝对路径: 'Link absolute path',
  目标绝对路径: 'Target absolute path',
  登记: 'Register',
  撤销: 'Revoke',
  用户授权可信链接与系统设置: 'Users, roots, trusted links and settings',
  '密码至少 10 位': 'Password (min 10 chars)',

  // Shared actions, states and errors
  '请求失败，请稍后重试': 'Request failed, please try again later',
  已复制到剪贴板: 'Copied to clipboard',
  '浏览器没有授予剪贴板权限，无法复制。':
    'The browser did not grant clipboard permission, so copying failed.',
  复制失败: 'Copy failed',
  操作失败: 'Operation failed',
  操作成功: 'Success',
  完成: 'Done',
  失败: 'Failed',
  处理中: 'In progress',
  '请稍候…': 'Please wait…',
  确定: 'OK',
  确认: 'Confirm',
  '输入 {code} 确认': 'Type {code} to confirm',
  关闭: 'Close',
  保存失败: 'Save failed',
  修改成功: 'Updated successfully',
  输入错误: 'Invalid input',
  权限: 'Permission',

  // Password dialog / password fields
  两次输入的新密码不一致: 'The new passwords do not match',
  修改密码失败: 'Failed to change password',
  修改密码: 'Change password',
  当前密码: 'Current password',
  新密码: 'New password',
  确认新密码: 'Confirm new password',
  '更新中…': 'Updating…',
  更新密码: 'Update password',

  // Quota dialog
  请输入有效的配额数字: 'Enter a valid quota number',
  配置配额: 'Set quota',
  '留空表示不限制；只统计用户 home 目录。':
    "Leave empty for unlimited; only the user's home directory is counted.",

  // Transfer dialog
  目录加载失败: 'Failed to load folders',
  '共 {count} 项，点击选择目标目录。': '{count} items; click to choose the destination folder.',
  上一级: 'Up',
  '当前目录没有子目录，可直接选择这里。':
    'This folder has no subfolders; you can choose it directly.',
  '目标目录：': 'Destination:',
  复制到这里: 'Copy here',
  移动到这里: 'Move here',

  // External / assigned paths
  添加失败: 'Failed to add',
  添加外部路径: 'Add external path',
  服务器目录: 'Server directory',
  别名: 'Alias',
  红盘: 'RedDisk',
  '目录必须已存在且可读；添加后到用户卡片的“分配外部路径”中授权。':
    'The directory must already exist and be readable; after adding, grant access via "Assign external path" on the user card.',
  '添加中…': 'Adding…',
  请选择外部路径: 'Select an external path',
  分配失败: 'Failed to assign',
  分配外部路径: 'Assign external path',
  已选择: 'Selected',
  '还没有外部路径，请先在外部路径区添加。':
    'No external paths yet. Add one in the external paths section first.',
  '分配中…': 'Assigning…',
  分配: 'Assign',
  外部路径: 'External paths',
  外部路径别名: 'External path alias',
  '还没有外部路径。': 'No external paths yet.',

  // New user dialog
  用户名和密码不能为空: 'Username and password are required',
  创建用户失败: 'Failed to create user',
  '初始密码（可修改）': 'Initial password (editable)',
  配额: 'Quota',
  '创建中…': 'Creating…',
  创建用户: 'Create user',

  // CSS picker
  '选择 CSS 文件': 'Select a CSS file',
  '选择自定义 CSS': 'Choose custom CSS',
  '自定义 CSS': 'Custom CSS',

  // Share dialog / edit
  创建分享失败: 'Failed to create share',
  '分享已创建，链接只显示一次：': 'Share created. The link is shown only once:',
  复制链接: 'Copy link',
  '密码（可选）': 'Password (optional)',
  留空表示无密码: 'Leave empty for no password',
  有效期: 'Expires',
  永久: 'Never',
  '1 天': '1 day',
  '7 天': '7 days',
  '30 天': '30 days',
  '90 天': '90 days',
  只读分享: 'Read-only share',
  创建分享: 'Create share',
  编辑共享: 'Edit share',

  // Share page
  根目录: 'Root',
  上传失败: 'Upload failed',

  // Shares page
  分享列表加载失败: 'Failed to load shares',
  修改分享失败: 'Failed to update share',
  生成分享地址失败: 'Failed to generate share link',
  '这里只显示你创建的分享。': 'Only shares you created are shown here.',
  '新密码，留空清除': 'New password; leave empty to clear',
  有密码: 'Password protected',
  无密码: 'No password',
  过期时间: 'Expires',
  启用共享: 'Enable share',
  停止共享: 'Stop share',
  复制地址: 'Copy link',
  '分享日期 {date}': 'Shared {date}',
  '访问次数 {count}': 'Visits {count}',
  '过期 {date}': 'Expires {date}',
  已停止: 'Stopped',
  '还没有分享。': 'No shares yet.',
  删除分享记录: 'Delete share record',
  '永久删除「{name}」的分享记录？': 'Permanently delete the share record for "{name}"?',
  删除分享失败: 'Failed to delete share',

  // Account menu
  账号菜单: 'Account menu',
  偏好设置: 'Preferences',
  系统管理: 'System administration',

  // Audit page
  审计日志加载失败: 'Failed to load audit log',
  审计日志: 'Audit log',
  '操作记录，滚动自动加载': 'Activity log; scroll to load more',
  返回管理: 'Back to admin',
  '暂无记录。': 'No records.',
  继续滚动加载: 'Scroll to load more',
  最近操作: 'Recent activity',
  '暂无审计记录。': 'No audit records.',

  // Avatar cropper
  调整头像: 'Adjust avatar',
  缩放: 'Zoom',
  使用这张头像: 'Use this avatar',
  头像: 'Avatar',

  // Preferences page
  偏好保存失败: 'Failed to save preferences',
  头像上传失败: 'Failed to upload avatar',
  密码已更新: 'Password updated',
  '账号、外观和文件浏览行为，修改后即时保存。':
    'Account, appearance and browsing behavior; changes are saved instantly.',
  账号: 'Account',
  外观: 'Appearance',
  主题: 'Theme',
  浅色: 'Light',
  深色: 'Dark',
  语言: 'Language',
  启用文本文件编辑: 'Enable text file editing',
  '系统设置已关闭文本编辑功能。': 'Text editing is disabled in system settings.',
  启用终端: 'Enable terminal',
  '系统设置已关闭终端功能。': 'The terminal is disabled in system settings.',
  未选择: 'Not selected',
  更换: 'Change',
  选择文件: 'Choose file',
  文件浏览: 'File browsing',
  显示隐藏文件: 'Show hidden files',
  显示缩略图: 'Show thumbnails',
  '系统设置已关闭缩略图功能。': 'Thumbnails are disabled in system settings.',
  上传同名文件: 'On filename conflict',
  自动重命名: 'Auto rename',
  覆盖: 'Overwrite',
  跳过: 'Skip',

  // Settings page
  加载分享失败: 'Failed to load shares',
  '加载 API Token 失败': 'Failed to load API tokens',
  '创建 API Token 失败': 'Failed to create API token',
  '撤销 API Token 失败': 'Failed to revoke API token',
  撤销分享失败: 'Failed to revoke share',
  '个人资料、偏好与共享管理': 'Profile, preferences and share management',
  设置分类: 'Settings sections',
  个人资料: 'Profile',
  共享设置: 'Sharing',
  'API Token': 'API Token',
  账号信息: 'Account information',
  角色: 'Role',
  文件列表: 'File list',
  '列表/网格视图与排序会按目录自动记忆。': 'List/grid view and sorting are remembered per folder.',
  'Token 只显示一次，请立即复制保存。read 只能读取，write 可修改文件。':
    'The token is shown only once; copy and save it now. read is read-only; write can modify files.',
  'Token 名称': 'Token name',
  '新 Token：': 'New token:',
  未使用: 'Never used',
  我的分享: 'My shares',
  '还没有分享。右键文件或目录即可创建分享。':
    'No shares yet. Right-click a file or folder to create one.',
  已撤销: 'Revoked',
  有效: 'Active',
  '撤销 API Token': 'Revoke API token',
  '撤销后该 Token 立即失效：{name}': 'The token stops working immediately after revocation: {name}',
  撤销分享: 'Revoke share',
  '撤销后链接立即失效：{path}': 'The link stops working immediately after revocation: {path}',

  // Admin panel
  配额必须是非负数字: 'Quota must be a non-negative number',
  移除存储目录: 'Remove storage directory',
  '删除「{alias}」？用户将无法再访问该目录。':
    'Delete "{alias}"? Users will no longer be able to access this directory.',
  删除存储目录: 'Delete storage directory',
  '删除「{alias}」？必须先移除所有用户授权。':
    'Delete "{alias}"? All user grants must be removed first.',
  '用户、存储目录、审计与系统设置': 'Users, storage directories, audit and system settings',
  系统设置: 'System settings',
  '密码（至少 10 位）': 'Password (at least 10 characters)',
  '配额字节，空=不限制': 'Quota in bytes, empty = unlimited',
  '{username} 角色': 'Role for {username}',
  重置密码: 'Reset password',
  分配存储目录: 'Assign storage directory',
  选择存储目录: 'Choose storage directory',
  用户显示名: 'User display name',
  '先登记服务器上已经存在且可读的存储目录（例如 /mnt/red → 红盘），再到用户页分配。目录不存在时请先在服务器上创建或挂载。':
    'First register an existing, readable storage directory on the server (for example /mnt/red → RedDisk), then assign it on the user page. If the directory does not exist, create or mount it on the server first.',
  '目录路径，例如 /mnt/red': 'Directory path, e.g. /mnt/red',
  '别名，例如 红盘': 'Alias, e.g. RedDisk',
  更新: 'Update',

  // Admin page
  管理数据加载失败: 'Failed to load admin data',
  从未登录: 'Never signed in',
  密码已重置: 'Password reset',
  角色切换失败: 'Failed to change role',
  状态切换失败: 'Failed to change status',
  删除用户失败: 'Failed to delete user',
  修改别名失败: 'Failed to update alias',
  删除外部路径失败: 'Failed to delete external path',
  权限修改失败: 'Failed to update permission',
  移除授权失败: 'Failed to remove grant',
  系统设置保存失败: 'Failed to save system settings',
  '用户、外部路径与系统设置': 'Users, external paths and system settings',
  成员: 'Member',
  已用: 'Used',
  最后登录: 'Last sign-in',
  缩略图: 'Thumbnails',
  文本编辑器: 'Text editor',
  默认显示隐藏文件: 'Show hidden files by default',
  默认主题: 'Default theme',
  跟随系统: 'Follow system',
  默认语言: 'Default language',
  默认配额: 'Default quota',
  删除用户: 'Delete user',
  '删除用户「{username}」？默认保留其 home 文件。':
    'Delete user "{username}"? Their home files are kept by default.',
  '+ 新建用户': '+ New user',
  '+ 分配外部路径': '+ Assign external path',
  '+ 添加外部路径': '+ Add external path',

  // Terminal
  不限: 'Unlimited',
  '当前目录：/{path}': 'Current directory: /{path}',
  '用户配额：{used} / {quota}': 'User quota: {used} / {quota}',
  '物理磁盘：{used} / {total}': 'Disk: {used} / {total}',
  '本目录：{dirs} 个目录，{files} 个文件': 'This folder: {dirs} folders, {files} files',
  无法读取终端信息: 'Unable to read terminal info',
  '终端连接尚未建立。': 'Terminal connection is not established yet.',
  '终端返回了无法解析的数据。': 'The terminal returned unreadable data.',
  '已建立 WebSocket 终端连接。': 'WebSocket terminal connected.',
  '终端连接已断开。': 'Terminal connection closed.',
  '已取消删除。': 'Deletion cancelled.',
  '确认删除？输入 y 确认，其他内容取消。': 'Delete? Type y to confirm, anything else cancels.',
  关闭终端: 'Close terminal',
  重连: 'Reconnect',

  // App shell, menus and progress
  'FileHub - 分享管理': 'FileHub - Share management',
  'FileHub - 设置': 'FileHub - Settings',
  'FileHub - 系统管理': 'FileHub - System administration',
  'FileHub - 审计日志': 'FileHub - Audit log',
  当前账号还没有可访问的目录: 'This account has no accessible directories',
  '计算中…': 'Calculating…',
  文件列表加载失败: 'Failed to load file list',
  '无法在浏览器中预览「{name}」。': '"{name}" cannot be previewed in the browser.',
  无法预览: 'Cannot preview',
  下载: 'Download',
  新名称: 'New name',
  目录名称: 'Folder name',
  '不能把目录移动到它自己里面。': 'A folder cannot be moved into itself.',
  移动失败: 'Move failed',
  移动文件: 'Moving files',
  复制文件: 'Copying files',
  '空文件暂不支持上传：{name}': 'Empty files cannot be uploaded: {name}',
  '{count} 个文件完成': '{count} files completed',
  '…以及另外 {count} 项': '…and {count} more items',
  永久删除: 'Permanently delete',
  '以下文件将被永久删除：\n{list}{more}':
    'The following files will be permanently deleted:\n{list}{more}',
  删除文件: 'Deleting files',
  '正在删除…': 'Deleting…',
  删除失败: 'Delete failed',
  '文件已被其他操作修改，请关闭后重新打开。':
    'The file was modified by another action; close and reopen it.',
  刚刚: 'Just now',
  '{count} 分钟前': '{count} minutes ago',
  '{count} 小时前': '{count} hours ago',
  '{count} 天前': '{count} days ago',
  正在加载: 'Loading',
  '索引中…': 'Indexing…',
  '已选 {count} 项': '{count} selected',
  取消选择: 'Clear selection',
  继续滚动加载更多: 'Scroll to load more',
  新建文件: 'New file',
  文件名称: 'File name',
  打开终端: 'Open terminal',
  排序方式: 'Sort by',
  列表: 'List',
  平铺: 'Tiles',
  视图切换: 'Switch view',
  '选择 {name}': 'Select {name}',
  '删除 {name}': 'Delete {name}',
  有未保存修改: 'Unsaved changes',
  已保存: 'Saved',
  '复制到…': 'Copy to…',
  '移动到…': 'Move to…',
  我的文件: 'My Files',
  '{username} 的新密码': 'New password for {username}',
  '{username} 配额': 'Quota for {username}',
  中文: 'Chinese',
  知道了: 'Got it',

  复位: 'Reset',
  显示预览图: 'Show preview',
  显示原图: 'Show original',
  关闭预览: 'Close preview',
  上一项: 'Previous',
  下一项: 'Next',
};

export function t(lang: Lang, key: string, vars?: Record<string, string | number>): string {
  let value = lang === 'en' ? (en[key] ?? key) : key;
  if (vars) {
    for (const [name, replacement] of Object.entries(vars)) {
      value = value.replaceAll(`{${name}}`, String(replacement));
    }
  }
  return value;
}

export function localeOf(lang: Lang): string {
  return lang === 'en' ? 'en-US' : 'zh-CN';
}

export function rootAlias(lang: Lang, alias: string): string {
  return alias === '我的文件' || alias === 'My Files' ? t(lang, '我的文件') : alias;
}

export function initLanguage(): void {
  let stored: string | null = null;
  try {
    stored = localStorage.getItem('filehub.lang');
  } catch {
    // ignore
  }
  const lang: Lang = stored === 'en' || stored === 'zh-CN' ? stored : 'en';
  language.set(lang);
  document.documentElement.lang = lang;
}

export function setLanguage(lang: Lang): void {
  language.set(lang);
  document.documentElement.lang = lang;
  try {
    localStorage.setItem('filehub.lang', lang);
  } catch {
    // ignore
  }
}
