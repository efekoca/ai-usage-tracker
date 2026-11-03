import { save } from '@tauri-apps/plugin-dialog'

/** A save dialog for one file type. Some dialogs leave the extension off; asking again with it lets the dialog warn before a file is replaced. */
export async function saveAs(defaultPath: string, name: string, ext: string): Promise<string | null> {
  const filters = [{ name, extensions: [ext] }]
  const chosen = await save({ defaultPath, filters })
  if (!chosen || chosen.toLowerCase().endsWith(`.${ext}`)) return chosen
  return save({ defaultPath: `${chosen}.${ext}`, filters })
}
