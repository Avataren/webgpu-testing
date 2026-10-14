-- Cached collision grid and heap-based A* on the shared authored dungeon geometry.
local W=require('world')
local N={rooms=W.rooms,links=W.links,cache={}}
local function contains(poly,x,z)
 local inside=false;local j=#poly
 for i=1,#poly do local a,b=poly[i],poly[j]
  if (a[2]>z)~=(b[2]>z) and x<(b[1]-a[1])*(z-a[2])/(b[2]-a[2])+a[1] then inside=not inside end;j=i
 end
 return inside
end
local function segment(x,z,a,b)
 local dx,dz=b[1]-a[1],b[2]-a[2];local t=math.max(0,math.min(1,((x-a[1])*dx+(z-a[2])*dz)/(dx*dx+dz*dz)))
 return (x-a[1]-t*dx)^2+(z-a[2]-t*dz)^2
end
local function exact(x,z)
 local floor=false;local cost=1
 for _,r in ipairs(W.rooms) do
  local dx,dz=x-r.x,z-r.z
  if contains(r.poly,dx,dz) and contains(r.poly,dx-.45,dz) and contains(r.poly,dx+.45,dz) and contains(r.poly,dx,dz-.45) and contains(r.poly,dx,dz+.45) then floor=true end
  for _,o in ipairs(r.obstacles) do
   if o.r and (dx-o.x)^2+(dz-o.z)^2<(o.r+.35)^2 then return false,1 end
   if o.w and math.abs(dx-o.x)<o.w/2+.35 and math.abs(dz-o.z)<o.d/2+.35 then return false,1 end
  end
  for _,h in ipairs(r.hazards) do if (dx-h.x)^2+(dz-h.z)^2<(h.r+.3)^2 then cost=4 end end
 end
 if not floor then for _,l in ipairs(W.links) do for i=2,#l.path do if segment(x,z,l.path[i-1],l.path[i])<(l.width/2-.55)^2 then floor=true;break end end;if floor then break end end end
 return floor,cost
end
local function key(x,z) return (x+512)*2048+z+1024 end
function N.cell(x,z)
 x,z=math.floor(x+.5),math.floor(z+.5);local k=key(x,z);local c=N.cache[k]
 if not c then local yes,cost=exact(x,z);c={yes=yes,cost=cost};N.cache[k]=c end
 return c.yes,c.cost
end
function N.walkable(x,z) return N.cell(x,z) end
function N.closest(x,z)
 local rx,rz=math.floor(x+.5),math.floor(z+.5)
 if N.cell(rx,rz) then return rx,rz end
 for rad=1,7 do local best,bx,bz=999,nil,nil
  for dx=-rad,rad do for dz=-rad,rad do if N.cell(rx+dx,rz+dz) and dx*dx+dz*dz<best then best=dx*dx+dz*dz;bx,bz=rx+dx,rz+dz end end end
  if bx then return bx,bz end
 end
 return rx,rz
end
local function push(heap,n)
 local i=#heap+1;heap[i]=n
 while i>1 do local p=math.floor(i/2);if heap[p].f<=n.f then break end;heap[i]=heap[p];i=p;heap[i]=n end
end
local function pop(heap)
 local n=heap[1];local last=table.remove(heap)
 if #heap>0 then heap[1]=last;local i=1
  while i*2<=#heap do local c=i*2;if c+1<=#heap and heap[c+1].f<heap[c].f then c=c+1 end;if heap[i].f<=heap[c].f then break end;heap[i],heap[c]=heap[c],heap[i];i=c end
 end
 return n
end
local dirs={{1,0},{-1,0},{0,1},{0,-1},{1,1},{1,-1},{-1,1},{-1,-1}}
function N.path(sx,sz,tx,tz)
 sx,sz=N.closest(sx,sz);tx,tz=N.closest(tx,tz)
 local start=key(sx,sz);local open={};push(open,{x=sx,z=sz,k=start,g=0,f=0})
 local best={[start]=0};local parent={};local nodes={};local closed={}
 for _=1,10000 do
  if #open==0 then break end;local p=pop(open)
  if p.x==tx and p.z==tz then local route={};local k=p.k
   while k~=start do local node=nodes[k];table.insert(route,1,{x=node.x,z=node.z});k=parent[k] end
   return route
  end
  if not closed[p.k] then closed[p.k]=true
   for _,d in ipairs(dirs) do local nx,nz=p.x+d[1],p.z+d[2];local k=key(nx,nz);local yes,cost=N.cell(nx,nz)
    if not closed[k] and yes and N.cell(p.x+d[1],p.z) and N.cell(p.x,p.z+d[2]) then
     local g=p.g+cost*((d[1]~=0 and d[2]~=0) and 1.414 or 1)
     if not best[k] or g<best[k] then best[k]=g;parent[k]=p.k;local n={x=nx,z=nz,k=k,g=g,f=g+math.sqrt((tx-nx)^2+(tz-nz)^2)};nodes[k]=n;push(open,n) end
    end
   end
  end
 end
 return {}
end
function N.move(a,tx,tz,dt,speed)
 local distance=math.sqrt((tx-a.x)^2+(tz-a.z)^2);a.moving=false
 if distance<.55 then return true end
 a.routeTimer=(a.routeTimer or 0)-dt
 if not a.route or a.routeTimer<=0 and (not a.gx or math.abs(tx-a.gx)+math.abs(tz-a.gz)>1) then a.route=N.path(a.x,a.z,tx,tz);a.routeIndex=1;a.routeTimer=.65;a.gx=tx;a.gz=tz end
 local p=a.route[a.routeIndex]
 if p and (p.x-a.x)^2+(p.z-a.z)^2<.17 then a.routeIndex=a.routeIndex+1;p=a.route[a.routeIndex] end
 local wx,wz=p and p.x or tx,p and p.z or tz
 local dx,dz=wx-a.x,wz-a.z;local len=math.sqrt(dx*dx+dz*dz)
 if len<.01 then return distance<.75 end
 local step=math.min(speed*dt,len);local nx,nz=a.x+dx/len*step,a.z+dz/len*step
 if N.walkable(nx,nz) then a.x,a.z=nx,nz;a.moving=true
 elseif N.walkable(nx,a.z) then a.x=nx;a.moving=true
 elseif N.walkable(a.x,nz) then a.z=nz;a.moving=true
 else a.route=nil end
 if a.moving then a.yaw=math.atan(-dx,-dz) end
 return distance<.75
end
function N.link(a,b)
 for _,l in ipairs(W.links) do if l.a==a and l.b==b then return l end end
end
return N
