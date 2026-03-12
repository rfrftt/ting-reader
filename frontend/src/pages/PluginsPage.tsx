import React, { useEffect, useState, useRef } from 'react';
import apiClient from '../api/client';
import type { Plugin, StorePlugin } from '../types';
import { 
  Puzzle, 
  Upload, 
  RefreshCw, 
  Trash2, 
  CheckCircle, 
  XCircle, 
  AlertCircle,
  ShoppingBag,
  Download,
  ArrowUpCircle,
  Search,
  Filter,
  // MoreVertical
} from 'lucide-react';

const PluginName = ({ name, className = "" }: { name: string, className?: string }) => {
  const [expanded, setExpanded] = useState(false);
  const [isOverflowing, setIsOverflowing] = useState(false);
  const ref = useRef<HTMLHeadingElement>(null);

  useEffect(() => {
    const checkOverflow = () => {
      if (ref.current && !expanded) {
        setIsOverflowing(ref.current.scrollWidth > ref.current.clientWidth);
      }
    };
    
    checkOverflow();
    
    const observer = new ResizeObserver(checkOverflow);
    if (ref.current) {
      observer.observe(ref.current);
    }
    
    return () => observer.disconnect();
  }, [name, expanded]);

  const handleClick = () => {
    if (expanded || isOverflowing) {
      setExpanded(!expanded);
    }
  };

  return (
    <h3 
      ref={ref}
      className={`${className} ${expanded ? 'break-words' : 'truncate'} ${(expanded || isOverflowing) ? 'cursor-pointer' : ''}`}
      onClick={handleClick}
      title={(expanded || isOverflowing) ? (expanded ? "Click to collapse" : "Click to expand") : undefined}
    >
      {name}
    </h3>
  );
};

const PluginsPage: React.FC = () => {
  const [activeTab, setActiveTab] = useState<'installed' | 'store' | 'updates'>('installed');
  const [plugins, setPlugins] = useState<Plugin[]>([]);
  const [storePlugins, setStorePlugins] = useState<StorePlugin[]>([]);
  const [loading, setLoading] = useState(true);
  const [storeLoading, setStoreLoading] = useState(false);
  const [uploading, setUploading] = useState(false);
  const [installingId, setInstallingId] = useState<string | null>(null);
  const [searchQuery, setSearchQuery] = useState('');
  const [filterType, setFilterType] = useState<string>('all');
  const [expandedDescriptions, setExpandedDescriptions] = useState<Set<string>>(new Set());

  const fileInputRef = useRef<HTMLInputElement>(null);

  const toggleDescription = (id: string) => {
    setExpandedDescriptions(prev => {
      const next = new Set(prev);
      if (next.has(id)) {
        next.delete(id);
      } else {
        next.add(id);
      }
      return next;
    });
  };



  const fetchPlugins = async () => {
    try {
      const response = await apiClient.get('/api/v1/plugins');
      setPlugins(response.data);
    } catch (err) {
      console.error('Failed to fetch plugins', err);
    } finally {
      setLoading(false);
    }
  };

  const fetchStorePlugins = async () => {
    setStoreLoading(true);
    try {
      const response = await apiClient.get('/api/v1/store/plugins');
      setStorePlugins(response.data);
    } catch (err) {
      console.error('Failed to fetch store plugins', err);
    } finally {
      setStoreLoading(false);
    }
  };

  useEffect(() => {
    fetchPlugins();
  }, []);

  useEffect(() => {
    if ((activeTab === 'store' || activeTab === 'updates') && storePlugins.length === 0) {
      fetchStorePlugins();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [activeTab]);

  const handleUpload = async (event: React.ChangeEvent<HTMLInputElement>) => {
    // ... existing upload logic ...
    const file = event.target.files?.[0];
    if (!file) return;

    const formData = new FormData();
    formData.append('file', file);

    setUploading(true);
    try {
      await apiClient.post('/api/v1/plugins/install', formData, {
        headers: {
          'Content-Type': 'multipart/form-data',
        },
      });
      fetchPlugins();
      alert('Plugin installed successfully!');
    } catch (err: unknown) {
      console.error('Failed to install plugin', err);
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      const msg = (err as any)?.response?.data?.error || (err as Error)?.message || 'Unknown error';
      alert(`Failed to install plugin: ${msg}`);
    } finally {
      setUploading(false);
      if (fileInputRef.current) {
        fileInputRef.current.value = '';
      }
    }
  };

  const handleInstallFromStore = async (pluginId: string) => {
    setInstallingId(pluginId);
    try {
      await apiClient.post('/api/v1/store/install', { pluginId });
      fetchPlugins();
      alert('Plugin installed successfully!');
    } catch (err: unknown) {
      console.error('Failed to install plugin from store', err);
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      const msg = (err as any)?.response?.data?.error || (err as Error)?.message || 'Unknown error';
      alert(`Failed to install plugin: ${msg}`);
    } finally {
      setInstallingId(null);
    }
  };

  const handleReload = async (id: string) => {
    // ... existing reload logic ...
    try {
      await apiClient.post(`/api/v1/plugins/${id}/reload`);
      fetchPlugins();
      alert('Plugin reloaded successfully!');
    } catch (err: unknown) {
      console.error('Failed to reload plugin', err);
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      const msg = (err as any)?.response?.data?.error || (err as Error)?.message || 'Unknown error';
      alert(`Failed to reload plugin: ${msg}`);
    }
  };

  const handleUninstall = async (id: string) => {
    // ... existing uninstall logic ...
    if (!confirm('Are you sure you want to uninstall this plugin?')) return;

    try {
      await apiClient.delete(`/api/v1/plugins/${id}`);
      fetchPlugins();
      alert('Plugin uninstalled successfully!');
    } catch (err: unknown) {
      console.error('Failed to uninstall plugin', err);
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      const msg = (err as any)?.response?.data?.error || (err as Error)?.message || 'Unknown error';
      alert(`Failed to uninstall plugin: ${msg}`);
    }
  };

  const getInstalledVersion = (pluginId: string) => {
    // Check for exact ID match first
    const exactMatch = plugins.find(p => p.id === pluginId);
    if (exactMatch) return exactMatch.version;

    // Check for ID@version format (legacy/backend format)
    const versionMatch = plugins.find(p => p.id.split('@')[0] === pluginId);
    return versionMatch ? versionMatch.version : null;
  };

  const isUpdateAvailable = (storePlugin: StorePlugin) => {
    const installedVersion = getInstalledVersion(storePlugin.id);
    if (!installedVersion) return false;
    // Simple version comparison (assumes vX.Y.Z format)
    return installedVersion.replace('v', '') < storePlugin.version.replace('v', '');
  };

  // Filter logic
  const getFilteredStorePlugins = () => {
    return storePlugins.filter(plugin => {
      // Filter out installed plugins for store tab
      if (activeTab === 'store' && getInstalledVersion(plugin.id)) {
        return false;
      }
      
      // Filter for updates tab
      if (activeTab === 'updates' && !isUpdateAvailable(plugin)) {
        return false;
      }

      // Search query
      if (searchQuery && !plugin.name.toLowerCase().includes(searchQuery.toLowerCase()) && 
          !plugin.description.toLowerCase().includes(searchQuery.toLowerCase())) {
        return false;
      }

      // Type filter
      if (filterType !== 'all' && plugin.pluginType !== filterType) {
        return false;
      }

      return true;
    });
  };

  const updateCount = storePlugins.filter(p => isUpdateAvailable(p)).length;

  if (loading && activeTab === 'installed') {
    return (
      <div className="flex items-center justify-center h-full">
        <div className="animate-spin rounded-full h-12 w-12 border-b-2 border-primary-600"></div>
      </div>
    );
  }

  return (
    <div className="flex-1 min-h-full flex flex-col p-4 sm:p-6 md:p-8 animate-in fade-in duration-500">
      <div className="flex items-center justify-between mb-8">
        <div>
          <h1 className="text-2xl md:text-3xl font-bold text-slate-900 dark:text-white flex items-center gap-3">
            <Puzzle size={28} className="text-primary-600 md:w-8 md:h-8" />
            插件管理
          </h1>
          <p className="text-sm md:text-base text-slate-500 dark:text-slate-400 mt-1">管理系统的扩展功能插件</p>
        </div>
        <div className="flex gap-3">
          <button 
            onClick={() => activeTab === 'installed' ? fetchPlugins() : fetchStorePlugins()} 
            className="p-2 text-slate-500 hover:text-primary-600 hover:bg-slate-100 dark:hover:bg-slate-800 rounded-xl transition-colors"
            title="Refresh"
          >
            <RefreshCw size={20} />
          </button>
          
          {activeTab === 'installed' && (
            <>
              <button 
                onClick={() => fileInputRef.current?.click()}
                disabled={uploading}
                className="flex items-center gap-2 bg-white dark:bg-slate-800 hover:bg-slate-50 dark:hover:bg-slate-700 text-slate-700 dark:text-slate-200 border border-slate-200 dark:border-slate-700 px-4 py-2 rounded-xl transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
              >
                {uploading ? (
                  <div className="animate-spin rounded-full h-4 w-4 border-b-2 border-slate-600"></div>
                ) : (
                  <Upload size={18} />
                )}
                <span>手动安装</span>
              </button>
              <input 
                type="file" 
                ref={fileInputRef} 
                onChange={handleUpload} 
                accept=".zip" 
                className="hidden" 
              />
            </>
          )}
        </div>
      </div>

      <div className="flex border-b border-slate-200 dark:border-slate-700 mb-6">
        <button
          className={`px-4 py-2 font-medium text-sm transition-colors relative ${
            activeTab === 'installed' 
              ? 'text-primary-600 dark:text-primary-400' 
              : 'text-slate-500 hover:text-slate-700 dark:hover:text-slate-300'
          }`}
          onClick={() => setActiveTab('installed')}
        >
          <div className="flex items-center gap-2">
            <Puzzle size={18} />
            <span>已安装</span>
            <span className="bg-slate-100 dark:bg-slate-800 text-slate-600 dark:text-slate-400 text-xs px-2 py-0.5 rounded-full">{plugins.length}</span>
          </div>
          {activeTab === 'installed' && (
            <div className="absolute bottom-0 left-0 right-0 h-0.5 bg-primary-600 dark:bg-primary-500"></div>
          )}
        </button>
        <button
          className={`px-4 py-2 font-medium text-sm transition-colors relative ${
            activeTab === 'store' 
              ? 'text-primary-600 dark:text-primary-400' 
              : 'text-slate-500 hover:text-slate-700 dark:hover:text-slate-300'
          }`}
          onClick={() => setActiveTab('store')}
        >
          <div className="flex items-center gap-2">
            <ShoppingBag size={18} />
            <span>插件商店</span>
          </div>
          {activeTab === 'store' && (
            <div className="absolute bottom-0 left-0 right-0 h-0.5 bg-primary-600 dark:bg-primary-500"></div>
          )}
        </button>
        <button
          className={`px-4 py-2 font-medium text-sm transition-colors relative ${
            activeTab === 'updates' 
              ? 'text-primary-600 dark:text-primary-400' 
              : 'text-slate-500 hover:text-slate-700 dark:hover:text-slate-300'
          }`}
          onClick={() => setActiveTab('updates')}
        >
          <div className="flex items-center gap-2">
            <ArrowUpCircle size={18} />
            <span>可升级</span>
            {updateCount > 0 && (
              <span className="bg-red-100 dark:bg-red-900/30 text-red-600 dark:text-red-400 text-xs px-2 py-0.5 rounded-full">{updateCount}</span>
            )}
          </div>
          {activeTab === 'updates' && (
            <div className="absolute bottom-0 left-0 right-0 h-0.5 bg-primary-600 dark:bg-primary-500"></div>
          )}
        </button>
      </div>

      {(activeTab === 'store' || activeTab === 'updates') && (
        <div className="mb-6 flex flex-col md:flex-row gap-4">
          <div className="relative flex-1">
            <Search className="absolute left-3 top-1/2 -translate-y-1/2 text-slate-400" size={18} />
            <input 
              type="text" 
              placeholder="搜索插件..." 
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
              className="w-full pl-10 pr-4 py-2 bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-700 rounded-xl focus:outline-none focus:ring-2 focus:ring-primary-500"
            />
          </div>
          <div className="relative">
             <div className="absolute left-3 top-1/2 -translate-y-1/2 text-slate-400 pointer-events-none">
               <Filter size={16} />
             </div>
             <select 
               value={filterType}
               onChange={(e) => setFilterType(e.target.value)}
               className="pl-10 pr-8 py-2 bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-700 rounded-xl focus:outline-none focus:ring-2 focus:ring-primary-500 appearance-none cursor-pointer"
             >
               <option value="all">所有类型</option>
               <option value="scraper">元数据刮削 (Scraper)</option>
               <option value="format">格式支持 (Format)</option>
               <option value="utility">工具 (Utility)</option>
             </select>
          </div>
        </div>
      )}

      {activeTab === 'installed' ? (
        plugins.length === 0 ? (
          <div className="flex-1 flex flex-col items-center justify-center text-slate-400 py-12">
            <Puzzle size={64} className="mb-4 opacity-50" />
            <p className="text-lg font-medium">暂无已安装的插件</p>
            <p className="text-sm mt-2">点击"插件商店"或"手动安装"添加新功能</p>
          </div>
        ) : (
          <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-6">
            {plugins.map((plugin) => (
              <div key={plugin.id} className="bg-white dark:bg-slate-900 rounded-2xl p-6 shadow-sm border border-slate-100 dark:border-slate-800 flex flex-col">
                <div className="flex items-start justify-between mb-4">
                  <div className="flex items-center gap-3 flex-1 min-w-0">
                    <div className={`w-10 h-10 rounded-xl flex items-center justify-center shrink-0 text-white ${
                      plugin.pluginType === 'scraper' ? 'bg-blue-500' : 
                      plugin.pluginType === 'format' ? 'bg-purple-500' : 'bg-green-500'
                    }`}>
                      <Puzzle size={20} />
                    </div>
                    <div className="min-w-0 flex-1">
                      <PluginName 
                        name={plugin.name} 
                        className="font-bold text-slate-900 dark:text-white"
                      />
                      <p className="text-xs text-slate-500 dark:text-slate-400">v{plugin.version}</p>
                    </div>
                  </div>
                  <div className="flex items-center gap-1 shrink-0 ml-2">
                    {plugin.state === 'active' ? (
                      <span className="flex items-center gap-1 text-[10px] uppercase font-bold text-green-600 bg-green-50 dark:bg-green-900/20 px-2 py-1 rounded-full border border-green-100 dark:border-green-900/30">
                        <CheckCircle size={12} /> Active
                      </span>
                    ) : plugin.state === 'failed' ? (
                      <span className="flex items-center gap-1 text-[10px] uppercase font-bold text-red-600 bg-red-50 dark:bg-red-900/20 px-2 py-1 rounded-full border border-red-100 dark:border-red-900/30">
                        <XCircle size={12} /> Failed
                      </span>
                    ) : (
                      <span className="flex items-center gap-1 text-[10px] uppercase font-bold text-slate-600 bg-slate-50 dark:bg-slate-800 px-2 py-1 rounded-full border border-slate-100 dark:border-slate-700">
                        <AlertCircle size={12} /> {plugin.state}
                      </span>
                    )}
                  </div>
                </div>
                
                <div className="flex-1 mb-4">
                  <div 
                    className={`text-sm text-slate-600 dark:text-slate-300 cursor-pointer ${
                      expandedDescriptions.has(plugin.id) ? '' : 'line-clamp-2'
                    }`}
                    onClick={() => toggleDescription(plugin.id)}
                    title={expandedDescriptions.has(plugin.id) ? "Click to collapse" : "Click to expand"}
                  >
                    {plugin.description}
                  </div>
                  <div className="mt-3 flex flex-wrap gap-2">
                    <span className="text-xs text-slate-500 bg-slate-50 dark:bg-slate-800 px-2 py-1 rounded-md border border-slate-100 dark:border-slate-700">
                      Type: {plugin.pluginType}
                    </span>
                    <span className="text-xs text-slate-500 bg-slate-50 dark:bg-slate-800 px-2 py-1 rounded-md border border-slate-100 dark:border-slate-700">
                      Author: {plugin.author}
                    </span>
                  </div>
                </div>

                <div className="pt-4 border-t border-slate-100 dark:border-slate-800 flex justify-end gap-2">
                  <button 
                    onClick={() => handleReload(plugin.id)}
                    className="p-2 text-slate-500 hover:text-blue-600 hover:bg-blue-50 dark:hover:bg-blue-900/20 rounded-lg transition-colors"
                    title="Reload"
                  >
                    <RefreshCw size={18} />
                  </button>
                  <button 
                    onClick={() => handleUninstall(plugin.id)}
                    className="p-2 text-slate-500 hover:text-red-600 hover:bg-red-50 dark:hover:bg-red-900/20 rounded-lg transition-colors"
                    title="Uninstall"
                  >
                    <Trash2 size={18} />
                  </button>
                </div>
              </div>
            ))}
          </div>
        )
      ) : (
        // Store or Updates Tab
        storeLoading ? (
          <div className="flex-1 flex items-center justify-center py-12">
            <div className="animate-spin rounded-full h-12 w-12 border-b-2 border-primary-600"></div>
          </div>
        ) : getFilteredStorePlugins().length === 0 ? (
          <div className="flex-1 flex flex-col items-center justify-center text-slate-400 py-12">
            <ShoppingBag size={64} className="mb-4 opacity-50" />
            <p className="text-lg font-medium">
              {activeTab === 'updates' ? '暂无可用更新' : '未找到符合条件的插件'}
            </p>
          </div>
        ) : (
          <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-6">
            {getFilteredStorePlugins().map((plugin) => {
              const installedVersion = getInstalledVersion(plugin.id);
              const hasUpdate = isUpdateAvailable(plugin);
              
              return (
                <div key={plugin.id} className="bg-white dark:bg-slate-900 rounded-2xl p-6 shadow-sm border border-slate-100 dark:border-slate-800 flex flex-col">
                  <div className="flex items-start justify-between mb-4">
                    <div className="flex items-center gap-3 flex-1 min-w-0">
                      <div className="w-10 h-10 rounded-xl flex items-center justify-center shrink-0 bg-slate-100 dark:bg-slate-800 text-2xl">
                        {plugin.icon || '🧩'}
                      </div>
                      <div className="min-w-0 flex-1">
                        <PluginName 
                          name={plugin.name} 
                          className="font-bold text-slate-900 dark:text-white"
                        />
                        <div className="flex items-center gap-2">
                           <p className="text-xs text-slate-500 dark:text-slate-400">{plugin.version}</p>
                           {installedVersion && (
                             <p className="text-xs text-slate-400 dark:text-slate-500 line-through">{installedVersion}</p>
                           )}
                        </div>
                      </div>
                    </div>
                    {hasUpdate && (
                       <span className="flex items-center gap-1 shrink-0 ml-2 text-[10px] uppercase font-bold text-green-600 bg-green-50 dark:bg-green-900/20 px-2 py-1 rounded-full border border-green-100 dark:border-green-900/30">
                         Update Available
                       </span>
                    )}
                  </div>
                  
                  <div className="flex-1 mb-4">
                    <div 
                      className={`text-sm text-slate-600 dark:text-slate-300 cursor-pointer ${
                        expandedDescriptions.has(plugin.id) ? '' : 'line-clamp-3'
                      }`}
                      onClick={() => toggleDescription(plugin.id)}
                      title={expandedDescriptions.has(plugin.id) ? "Click to collapse" : "Click to expand"}
                    >
                      {plugin.description}
                    </div>
                    <div className="mt-3 flex flex-wrap gap-2">
                      <span className="text-xs text-slate-500 bg-slate-50 dark:bg-slate-800 px-2 py-1 rounded-md border border-slate-100 dark:border-slate-700">
                        {plugin.pluginType}
                      </span>
                    </div>
                  </div>

                  <div className="pt-4 border-t border-slate-100 dark:border-slate-800 flex justify-end gap-2">
                    {plugin.repo && (
                      <a 
                        href={`https://github.com/${plugin.repo}`}
                        target="_blank"
                        rel="noopener noreferrer"
                        className="p-2 text-slate-500 hover:text-slate-900 dark:hover:text-white hover:bg-slate-100 dark:hover:bg-slate-800 rounded-lg transition-colors mr-auto"
                        title="View Source"
                      >
                        <svg viewBox="0 0 24 24" width="18" height="18" stroke="currentColor" strokeWidth="2" fill="none" strokeLinecap="round" strokeLinejoin="round"><path d="M9 19c-5 1.5-5-2.5-7-3m14 6v-3.87a3.37 3.37 0 0 0-.94-2.61c3.14-.35 6.44-1.54 6.44-7A5.44 5.44 0 0 0 20 4.77 5.07 5.07 0 0 0 19.91 1S18.73.65 16 2.48a13.38 13.38 0 0 0-7 0C6.27.65 5.09 1 5.09 1A5.07 5.07 0 0 0 5 4.77a5.44 5.44 0 0 0-1.5 3.78c0 5.42 3.3 6.61 6.44 7A3.37 3.37 0 0 0 9 18.13V22"></path></svg>
                      </a>
                    )}
                    
                    <button 
                      onClick={() => handleInstallFromStore(plugin.id)}
                      disabled={installingId === plugin.id}
                      className={`flex items-center gap-2 px-4 py-2 rounded-xl transition-colors ${
                        installingId === plugin.id
                          ? 'bg-slate-100 dark:bg-slate-800 text-slate-400 cursor-not-allowed'
                          : hasUpdate
                            ? 'bg-green-600 hover:bg-green-700 text-white'
                            : 'bg-primary-600 hover:bg-primary-700 text-white'
                      }`}
                    >
                      {installingId === plugin.id ? (
                        <div className="animate-spin rounded-full h-4 w-4 border-b-2 border-current"></div>
                      ) : (
                        <Download size={18} />
                      )}
                      <span>
                        {hasUpdate ? '更新' : '安装'}
                      </span>
                    </button>
                  </div>
                </div>
              );
            })}
          </div>
        )
      )}
    </div>
  );
};

export default PluginsPage;
