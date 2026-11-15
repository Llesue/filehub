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
};

export function t(lang: Lang, key: string): string {
  return lang === 'en' ? (en[key] ?? key) : key;
}

export function initLanguage(): void {
  let stored: string | null = null;
  try {
    stored = localStorage.getItem('filehub.lang');
  } catch {
    // ignore
  }
  const lang: Lang =
    stored === 'en' || stored === 'zh-CN'
      ? stored
      : navigator.language.toLowerCase().startsWith('zh')
        ? 'zh-CN'
        : 'en';
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
