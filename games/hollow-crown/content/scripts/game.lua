-- The Hollow Crown: a branching expedition, equipment, resource attrition, and danger.
local W=require('world');local NAV=require('navigation');local LOOT=require('loot');local HUD=require('hud')
local PI=math.pi
local model,fx={},{shots={},rings={}}
local palette={knight='#91c7dc',ranger='#b4c37d',mage='#bb9be8',enemy='#e99465'}
local function rng(lo,hi)
 GAME.rng=(GAME.rng*1664525+1013904223)%4294967296
 local v=GAME.rng/4294967296
 if not lo then return v end
 return math.floor(lo+v*(hi-lo+1))
end
local function dist(a,b) return math.sqrt((a.x-b.x)^2+(a.z-b.z)^2) end
local function vis(id,on) if GAME.render and id then set_component(id,'Visible',on) end end
local function message(text,color)
 table.insert(GAME.journal,1,{text=text,color=color or '#c4bdab',at=GAME.t})
 while #GAME.journal>6 do table.remove(GAME.journal) end
end
local function sound(name,volume,pitch)
 if GAME.render and not GAME.muted then play_sound('content/audio/'..name..'.wav',{volume=volume or .4,pitch=pitch or 1}) end
end
local function burst(a,color,count)
 if GAME.render then emit_particles(a.x,1.1,a.z,{count=count or 22,color=color,intensity=1.4,speed={.6,2.3},life={.25,.6},size=.07,gravity=-2}) end
end
local function floating(a,text,color)
 if GAME.render then table.insert(GAME.floats,{x=a.x,z=a.z,y=a.boss and 3.4 or 2.2,text=text,color=color,life=1.2}) end
end
local function living()
 local n=0;for _,h in ipairs(GAME.heroes) do if h.hp>0 then n=n+1 end end;return n
end
local function enemiesLeft()
 local n=0;for _,e in ipairs(GAME.enemies) do if e.active and e.hp>0 then n=n+1 end end;return n
end
local function rank() return math.min(5,math.ceil(GAME.stop/3)) end
local function applyAppearance(h)
 if not GAME.render then return end
 local prefix=({'Knight','Ranger','Mage'})[h.index];local item=h.gear.weapon
 local col=item.color
 if h.class=='knight' then
  vis(model.KnightAxe,item.kind=='axe');vis(model.KnightSword,item.kind~='axe');vis(model.KnightHilt,item.kind~='axe')
  set_material(model.KnightSword,{color=col,metallic=.8,roughness=.3,emissive=item.tier>=3 and .15 or 0})
  set_material(model.KnightAxe,{color=col,metallic=.8,roughness=.3})
 elseif h.class=='ranger' then
  vis(model.RangerCrossbow,item.kind=='crossbow');vis(model.RangerBow,item.kind~='crossbow');vis(model.RangerBowstring,item.kind~='crossbow')
  set_material(model.RangerBow,{color=col,roughness=.5});set_material(model.RangerCrossbow,{color=col,roughness=.5})
 else
  vis(model.MageRunes,item.tier>=2);set_material(model.MageStaffcrystal,{color=item.tier>=3 and '#9c68da' or '#118bc9',emissive=1.4,unlit=true})
 end
 local armor=h.gear.armor
 set_material(model[prefix..'Body'],{color=armor.tier>0 and armor.color or ({'#aabdc4','#456443','#493a70'})[h.index],metallic=h.class=='knight' and .7 or .15,roughness=.6})
end
local function nearestEnemy(h)
 local target,best=nil,999
 for _,e in ipairs(GAME.enemies) do if e.active and e.hp>0 then
  local d=dist(h,e);local score=d-(e.ranged and d<9 and 2 or 0)
  if score<best then best=score;target=e end
 end end
 return target,target and dist(h,target) or 999
end
local function enemyTarget(e)
 local target,best=nil,999
 for _,h in ipairs(GAME.heroes) do if h.hp>0 then
  local d=dist(h,e);local score=d
  if e.ranged then score=d*.25+h.hp/h.maxhp*3 end
  if e.kind=='spider' and h.class=='mage' then score=score-2 end
  if score<best then target,best=h,score end
 end end
 return target,target and dist(e,target) or 999
end
local function heal(h,amount)
 if h.hp<=0 then return end
 local gain=math.min(amount,h.maxhp-h.hp);h.hp=h.hp+gain
 if gain>0 then floating(h,'+'..math.floor(gain),'#a7d3a0');burst(h,'#90c994',18) end
end
local function hurt(target,amount,color)
 if target.hp<=0 then return end
 if target.hero then amount=math.floor(amount*(1-target.resist)*(target.guard>0 and .6 or 1)) end
 amount=math.max(1,amount);target.hp=math.max(0,target.hp-amount)
 floating(target,tostring(amount),color);burst(target,color,12)
 if target.hp==0 then
  target.deadAt=GAME.t;target.route=nil
  if target.hero then GAME.fallen=GAME.fallen+1;GAME.firstFall=GAME.firstFall or {room=GAME.room,name=target.name,time=GAME.elapsed};message(target.name..' falls. There is no free resurrection.','#db8e7c')
  else GAME.kills=GAME.kills+1;GAME.gold=GAME.gold+rng(3,7)+(target.boss and 40 or 0);message(target.name..' is defeated.','#c4ac75');sound('bone',.45) end
 end
end
local function poison(h,seconds,damage)
 if h.hp>0 then h.poison=math.max(h.poison,seconds);h.poisonDamage=math.max(h.poisonDamage or 0,damage) end
end
local function attack(a,b,damage,bonus)
 local roll=rng(1,20);local ac=b.ac-((b.sunder or 0)>0 and 2 or 0)
 local critical=roll==20 or a.hero and a.crit>=3 and roll==19
 local hit=critical or roll~=1 and roll+bonus>=ac
 GAME.rolls=GAME.rolls+1;GAME.lastRoll={die=roll,bonus=bonus,result=hit and (critical and 'CRITICAL' or 'HIT') or 'MISS',name=a.name}
 if hit then
  if critical then damage=damage*2;GAME.crits=GAME.crits+1;message(a.name..(roll==20 and ' rolls a natural 20!' or ' lands an enchanted critical!'),'#dfc477') end
  hurt(b,damage,a.hero and palette[a.class] or palette.enemy)
 else floating(b,'miss','#8f9da4') end
 return hit
end
local function shot(a,b,kind)
 if not GAME.render then return end
 local s=GAME.shots[GAME.nextShot];GAME.nextShot=GAME.nextShot%#GAME.shots+1
 s.active=true;s.life=.35;s.ax=a.x;s.az=a.z;s.bx=b.x;s.bz=b.z
 set_mesh(s.id,kind=='arrow' and 'cylinder' or 'sphere');set_scale(s.id,kind=='arrow' and .025 or .14,kind=='arrow' and .025 or .14,kind=='arrow' and .8 or .14)
 set_material(s.id,{color=kind=='arrow' and '#c8b38b' or a.hero and '#5593cf' or '#9766be',emissive=kind=='arrow' and .1 or 1.2,unlit=true});vis(s.id,true)
end
local function potion(force)
 if GAME.potions<=0 or GAME.potionCd>0 then return false end
 local weak=nil;for _,h in ipairs(GAME.heroes) do if h.hp>0 and (not weak or h.hp/h.maxhp<weak.hp/weak.maxhp) then weak=h end end
 if weak and (force or weak.hp/weak.maxhp<.34 or weak.poison>3 and weak.hp<50) then
  GAME.potions=GAME.potions-1;GAME.potionsUsed=GAME.potionsUsed+1;GAME.potionCd=8;heal(weak,42);weak.poison=0;sound('treasure',.35,.8);message(weak.name..' uses a healing draught. '..GAME.potions..' remain.','#abc595');return true
 end
 return false
end
local function nova(force)
 local h=GAME.heroes[3];if h.hp<=0 or h.mana<20 or h.powerCd>0 then return false end
 local e,d=nearestEnemy(h);if not e or d>8 then return false end
 local count=0;for _,t in ipairs(GAME.enemies) do if t.active and t.hp>0 and dist(t,e)<2.7 then count=count+1 end end
 if not force and count<2 and not e.boss then return false end
 h.mana=h.mana-20;h.powerCd=11;h.swing=.45
 if GAME.render then emit_particles(e.x,.2,e.z,{count=70,shape='ring',color='#668fca',intensity=1.5,speed=4,life=.65,size=.1}) end
 for _,t in ipairs(GAME.enemies) do if t.active and t.hp>0 and dist(t,e)<2.7 then hurt(t,rng(13,19)+math.floor(h.damage*.5),'#81a8da');t.slow=3 end end
 sound('arcane',.4);message('Ilyra spends mana on frost nova.','#aba7dc');return true
end
local archetypes={
 skeleton={hp=30,damage=5,bonus=3,ac=11,cd=2.8,speed=1.7,name='Bone sentinel'},
 archer={hp=27,damage=7,bonus=4,ac=11,cd=3.1,speed=1.6,ranged=true,range=10,name='Crypt bowman'},
 cultist={hp=36,damage=7,bonus=4,ac=12,cd=3.2,speed=1.4,ranged=true,range=8.5,name='Grave herald'},
 ghoul={hp=40,damage=6,bonus=4,ac=12,cd=2.5,speed=2.5,name='Carrion ghoul'},
 spider={hp=31,damage=5,bonus=4,ac=12,cd=2.4,speed=2.6,name='Silk widow'},
 abbot={hp=140,damage=14,bonus=5,ac=14,cd=3,speed=1.8,boss=true,name='The Bone Abbot'},
 oracle={hp=170,damage=10,bonus=5,ac=14,cd=3.4,speed=1.5,ranged=true,range=10,boss=true,name='The Glass Oracle'},
 warden={hp=175,damage=16,bonus=6,ac=16,cd=3.2,speed=1.5,boss=true,name='Iron Confessor'},
 king={hp=270,damage=19,bonus=7,ac=17,cd=3,speed=1.7,boss=true,name='The Unburied King'}
}
local pools={skeleton={1,2,3,4,5,6},archer={7,8},cultist={9,14,15},ghoul={11,16},spider={12,13},abbot={17},oracle={18},warden={19},king={10}}
local function spawnEnemy(kind,index,ambush)
 local slot=pools[kind][index or 1];local e=GAME.enemies[slot];local def=archetypes[kind]
 local r=W.rooms[GAME.room];local tier=rank()
 e.kind=kind;e.name=def.name;e.boss=def.boss;e.ranged=def.ranged;e.range=def.range or (e.boss and 2.5 or 1.5);e.ac=def.ac+math.floor(tier/3)
 e.hp=math.floor((def.hp+tier*(e.boss and 9 or 3))*GAME.pressure);e.maxhp=e.hp;e.damage=def.damage+math.floor(tier*1.25);e.bonus=def.bonus+math.floor(tier/3);e.speed=def.speed;e.attackCd=def.cd
 e.active=true;e.cd=1+rng();e.abilityCd=5+rng()*3;e.slow=0;e.swing=0;e.summoned=false;e.deadAt=nil;e.route=nil
 local tries=0
 repeat e.x=r.x+rng(-6,6);e.z=r.z-rng(3,math.max(5,math.floor(r.span*.3)));tries=tries+1 until NAV.walkable(e.x,e.z) or tries>50
 if not NAV.walkable(e.x,e.z) then e.x,e.z=NAV.closest(r.x,r.z-2) end
 if ambush then e.x,e.z=NAV.closest(GAME.leader.x+rng(-4,4),GAME.leader.z+rng(-4,4)) end
 e.yaw=0;vis(e.id,true)
 if GAME.render then set_translation(e.id,e.x,0,e.z);set_rotation(e.id,0,0,0) end
 return e
end
local encounters={
 [1]={'skeleton','skeleton','skeleton'},[2]={'skeleton','skeleton','archer'},[3]={'cultist','skeleton','skeleton'},[4]={'spider','spider','ghoul'},
 [5]={'skeleton','skeleton','skeleton','skeleton','archer'},[6]={'abbot','skeleton','skeleton'},[7]={'ghoul','ghoul','spider','spider'},[8]={},
 [9]={'ghoul','skeleton','archer'},[10]={'cultist','cultist','skeleton','skeleton'},[11]={'oracle','cultist','skeleton'},
 [12]={'archer','archer','skeleton','skeleton'},[13]={'skeleton','skeleton','cultist','ghoul'},[14]={'ghoul','skeleton','archer','spider'},[15]={'warden','skeleton','skeleton'},[16]={'king','cultist','skeleton'}
}
local function reveal(extra)
 if not GAME.render then return end
 GAME.visibleWorld={}
 for i,id in ipairs(model.rooms) do local on=i==GAME.room or i==extra;vis(id,on);GAME.visibleWorld['Room'..i]=on end
 for i,id in ipairs(model.corridors) do local l=W.links[i];local on=(l.a==GAME.room or l.b==GAME.room) and (not extra or l.a==extra or l.b==extra);vis(id,on);GAME.visibleWorld['Corridor'..i]=on end
end
local function activateRoom(n)
 GAME.room=n;GAME.stop=GAME.stop+1;GAME.visited[n]=true;GAME.phase='intro';GAME.phaseTime=0;GAME.banner=4;GAME.target=nil;GAME.hazards={};GAME.lootGiven=false;GAME.ambushed=false;GAME.lootPause=0
 for _,e in ipairs(GAME.enemies) do e.active=false;vis(e.id,false) end
 local counts={}
 for _,kind in ipairs(encounters[n]) do counts[kind]=(counts[kind] or 0)+1;spawnEnemy(kind,counts[kind]) end
 if n>1 and n~=8 and rng()<.25 and (counts.skeleton or 0)<6 then spawnEnemy('skeleton',(counts.skeleton or 0)+1) end
 for _,h in ipairs(GAME.heroes) do h.route=nil end
 reveal();message(W.rooms[n].name..' — '..W.rooms[n].subtitle,'#c3b184')
end
local nextRoom={[1]=2,[3]=5,[4]=5,[5]=6,[6]=7,[7]=8,[9]=11,[10]=11,[11]=12,[12]=13,[13]=14,[14]=15,[15]=16}
local function depart(n)
 GAME.nextRoom=n;GAME.link=NAV.link(GAME.room,n);GAME.waypoint=1;GAME.phase='travel';GAME.phaseTime=0;GAME.travelTrap=false;GAME.travelHazardAt=6+rng()*6
 for _,h in ipairs(GAME.heroes) do h.route=nil end
 reveal(n);message('The company takes the passage to '..W.rooms[n].name..'.','#aab6b7')
end
local function equipmentDrop(hint,slotHint)
 local item=LOOT.roll(rng,math.min(4,1+math.floor(GAME.stop/4)),hint,slotHint)
 local equipped=false
 for _,h in ipairs(GAME.heroes) do if h.hp>0 and LOOT.equip(h,item) then
  equipped=true;GAME.upgrades=GAME.upgrades+1;applyAppearance(h);message(h.name..' equips '..item.name..' ('..item.rarity..').',item.color);break
 end end
 if not equipped then GAME.gold=GAME.gold+item.value;message(item.name..' is salvaged for '..item.value..' gold.','#949e9c') end
 GAME.lootCount=GAME.lootCount+1;GAME.lastLoot={name=item.name,tier=item.tier,slot=item.slot,class=item.class,equipped=equipped};table.insert(GAME.lootHistory,1,GAME.lastLoot)
 while #GAME.lootHistory>20 do table.remove(GAME.lootHistory) end
end
local function reward()
 local r=W.rooms[GAME.room];GAME.lootGiven=true;vis(model.chests[GAME.room],false)
 GAME.gold=GAME.gold+rng(18,35)
 equipmentDrop();if GAME.stop%3==0 or r.theme=='stash' then equipmentDrop() end
 if r.theme=='stash' then GAME.potions=GAME.potions+2;message('Two smuggled healing draughts survived.','#afbd91') end
 if r.theme=='forge' then
  local h=GAME.heroes[1];for _,a in ipairs(GAME.heroes) do if a.hp>0 and a.gear.weapon.tier<h.gear.weapon.tier then h=a end end
  equipmentDrop(h.class,'weapon')
 end
 if rng()<.26 then GAME.potions=GAME.potions+1;message('A sealed healing draught survives in the cache.','#b3c198') end
 if r.theme=='well' then
  for _,h in ipairs(GAME.heroes) do heal(h,46);h.mana=math.min(h.maxmana,h.mana+45);h.poison=0 end;message('Moonwater mends the living. The fallen remain still.','#85b6ba')
 elseif r.theme=='chapel' then
  local fallen=nil;for _,h in ipairs(GAME.heroes) do if h.hp<=0 then fallen=h;break end end
  if fallen and GAME.gold>=90 then GAME.gold=GAME.gold-90;fallen.hp=40;fallen.poison=0;message('Ninety gold restores '..fallen.name..' to the light.','#c7b683')
  elseif GAME.gold>=45 then GAME.gold=GAME.gold-45;for _,h in ipairs(GAME.heroes) do heal(h,20) end;message('A costly blessing steadies the company.','#b7bc9b') end
 end
 sound('treasure',.5);GAME.lootPause=5
end
local function finish(outcome)
 GAME.phase=outcome;GAME.phaseTime=0;GAME.result=outcome;GAME.finishedAt=GAME.elapsed
 GAME[outcome=='victory' and 'victories' or 'defeats']=GAME[outcome=='victory' and 'victories' or 'defeats']+1
 local record={outcome=outcome,seed=GAME.seed,time=GAME.elapsed,room=GAME.room,stops=GAME.stop,kills=GAME.kills,upgrades=GAME.upgrades,pressure=GAME.pressure,route=table.concat(GAME.routeLog,',')}
 table.insert(GAME.history,1,record);while #GAME.history>12 do table.remove(GAME.history) end
 message(outcome=='victory' and 'The Crown is claimed by those who lived to reach it.' or 'The expedition is lost. The abbey keeps their gear.','#d6b581')
 if outcome=='victory' then vis(model.crown,false) end
end
local function makeState(seed,prior,render)
 local g={version=2,seed=seed,rng=seed,t=0,elapsed=0,room=1,stop=0,auto=true,paused=false,speed=1,muted=false,render=render,solo=false,heroes={},enemies={},shots=fx.shots,nextShot=1,floats={},hazards={},journal={},visited={},routeLog={},lootHistory={},gold=0,kills=0,rolls=0,crits=0,potions=4,potionCd=0,potionsUsed=0,upgrades=0,lootCount=0,fallen=0,hazardHits=0,trapsDetected=0,history=prior and prior.history or {},victories=prior and prior.victories or 0,defeats=prior and prior.defeats or 0,expeditions=prior and prior.expeditions+1 or 1,camx=0,camz=8,camzoom=1.12,lightTimer=0,fxTimer=0,environmentTimer=0,inventory=false,map=false,orbit=false}
 GAME=g;g.pressure=.84+rng()*.4
 for i,n in ipairs({'Knight','Ranger','Mage'}) do
  local class=({'knight','ranger','mage'})[i];local h={index=i,id=model[n],weapon=model[n..'Weapon'],left=model[n..'LegL'],right=model[n..'LegR'],name=({'Ser Aldric','Thorne','Ilyra'})[i],class=class,hero=true,x=(i-2)*1.5,z=10+i*.5,yaw=0,basehp=({135,95,85})[i],baseac=({16,13,12})[i],hp=({135,95,85})[i],mana=65,cd=0,powerCd=0,swing=0,guard=0,guardCd=0,poison=0,poisonTick=0,slow=0,sunder=0,gear=LOOT.starter(class)}
  LOOT.derive(h);g.heroes[i]=h;applyAppearance(h)
 end
 g.leader=g.heroes[1]
 for i=1,19 do g.enemies[i]={id=model['Enemy'..i],weapon=model['Enemy'..i..'Weapon'],active=false,hp=0,x=0,z=0,yaw=0,swing=0} end
 for _,id in ipairs(model.chests) do vis(id,true) end;vis(model.crown,true)
 activateRoom(1);g.routeLog={1};message('Expedition '..g.expeditions..'. Supplies are scarce; luck is uncertain.','#cbb685')
 return g
end
local function hazard(x,z,radius,damage,kind,delay)
 local h={x=x,z=z,r=radius,damage=damage,kind=kind,timer=0,delay=delay or 1.4,reaction={}}
 for i=1,3 do h.reaction[i]=.45+rng()*.8 end
 table.insert(GAME.hazards,h)
end
local function dodge(h,dt)
 for _,t in ipairs(GAME.hazards) do
  if dist(h,t)<t.r+1 and t.timer>t.reaction[h.index] and h.swing<.15 then
   local dx,dz=h.x-t.x,h.z-t.z;local len=math.sqrt(dx*dx+dz*dz)
   if len<.2 then dx,dz=1,0;len=1 end
   NAV.move(h,t.x+dx/len*(t.r+1.4),t.z+dz/len*(t.r+1.4),dt,h.speed*1.3);return true
  end
 end
 return false
end
local function updateHeroes(dt)
 local leader=GAME.heroes[1];if leader.hp<=0 then for _,h in ipairs(GAME.heroes) do if h.hp>0 then leader=h;break end end end;GAME.leader=leader
 for i,h in ipairs(GAME.heroes) do
  h.cd=math.max(0,h.cd-dt);h.powerCd=math.max(0,h.powerCd-dt);h.swing=math.max(0,h.swing-dt);h.slow=math.max(0,h.slow-dt);h.sunder=math.max(0,h.sunder-dt);h.guard=math.max(0,h.guard-dt);h.guardCd=math.max(0,h.guardCd-dt);h.moving=false
  h.mana=math.min(h.maxmana,h.mana+dt*(GAME.phase=='fight' and h.regen or h.regen+1))
  if h.hp>0 then
   if h.poison>0 then h.poison=h.poison-dt;h.poisonTick=h.poisonTick+dt;if h.poisonTick>=1 then h.poisonTick=0;hurt(h,h.poisonDamage or 2,'#91ad62') end else h.poisonDamage=0 end
   local enemy,d=nearestEnemy(h);local fighting=GAME.phase=='fight' and enemy~=nil;local direct=not GAME.auto and h==leader
   local speed=h.speed*(h.slow>0 and .6 or 1);local evading=false
   if GAME.auto then evading=dodge(h,dt) end
   if direct then
    local dx,dz=0,0
    if is_key_pressed('W') or is_key_pressed('ArrowUp') then dx=dx-.6;dz=dz-.8 end
    if is_key_pressed('S') or is_key_pressed('ArrowDown') then dx=dx+.6;dz=dz+.8 end
    if is_key_pressed('D') or is_key_pressed('ArrowRight') then dx=dx+.8;dz=dz-.6 end
    if is_key_pressed('A') or is_key_pressed('ArrowLeft') then dx=dx-.8;dz=dz+.6 end
    if dx~=0 or dz~=0 then GAME.target=nil;h.route=nil;local len=math.sqrt(dx*dx+dz*dz);local nx,nz=h.x+dx/len*dt*speed,h.z+dz/len*dt*speed
     if NAV.walkable(nx,nz) then h.x,h.z=nx,nz;h.moving=true;h.yaw=math.atan(-dx,-dz) end
    elseif GAME.target then if NAV.move(h,GAME.target.x,GAME.target.z,dt,speed) then GAME.target=nil end end
   elseif not evading then
    if fighting then
     local range=h.class=='knight' and 1.8 or h.class=='ranger' and 9 or 8
     if d>range then NAV.move(h,enemy.x,enemy.z,dt,speed)
     elseif d<2.4 and h.class~='knight' then local dx,dz=h.x-enemy.x,h.z-enemy.z;local len=math.max(.01,d);NAV.move(h,h.x+dx/len*3,h.z+dz/len*3,dt,speed) end
    elseif h==leader and GAME.auto then
     local r=W.rooms[GAME.room]
     if GAME.phase=='loot' then NAV.move(h,r.x+r.loot[1]-1.6,r.z+r.loot[2]+1.3,dt,speed)
     elseif GAME.phase=='travel' then local p=GAME.link.path[GAME.waypoint];if p then NAV.move(h,p[1],p[2],dt,speed) end
     elseif GAME.phase=='intro' then NAV.move(h,r.x+r.battle[1],r.z+r.battle[2]+4,dt,speed*.8) end
    elseif h~=leader and dist(h,leader)>2.2 then NAV.move(h,leader.x+(i==2 and -1 or 1),leader.z+1.5,dt,speed*1.12) end
   end
   if fighting and not evading and h.hp>0 then
    h.yaw=math.atan(h.x-enemy.x,h.z-enemy.z)
    local range=h.class=='knight' and 2 or h.class=='ranger' and 9.5 or 8.5
    if h.cd<=0 and d<range and (not direct or is_key_pressed('Space')) then
     h.swing=.45;h.cd=({1.65,2.05,2.5})[i]*h.cooldown
     local damage=({9,8,10})[i]+h.damage+rng(-2,2)
     if h.class=='mage' then if h.mana>=2 then h.mana=h.mana-2 else damage=math.max(4,damage-4) end end
     if h.class~='knight' then shot(h,enemy,h.class=='ranger' and 'arrow' or 'magic') end
     attack(h,enemy,damage,h.hit);sound(h.class=='mage' and 'arcane' or 'steel',.3)
    end
    if h.class=='knight' and GAME.auto and h.hp/h.maxhp<.35 and h.guardCd<=0 then h.guard=2.7;h.guardCd=10;message('Aldric braces behind his shield.','#91b5c6') end
   end
  end
 end
 if GAME.auto and (GAME.phase=='fight' or GAME.phase=='travel') then potion(false) end
 if GAME.auto and GAME.phase=='fight' then nova(false) end
end
local function updateEnemies(dt)
 for _,e in ipairs(GAME.enemies) do if e.active and e.hp>0 then
  e.cd=e.cd-dt;e.swing=math.max(0,e.swing-dt);e.slow=math.max(0,e.slow-dt);e.moving=false
  if GAME.phase=='fight' then
   local h,d=enemyTarget(e)
   if h then
    if d>e.range then NAV.move(e,h.x,h.z,dt,e.speed*(e.slow>0 and .5 or 1)) end
    e.yaw=math.atan(e.x-h.x,e.z-h.z)
    if d<e.range+.2 and e.cd<=0 then
     e.swing=.45;e.cd=e.attackCd*(.95+rng()*.15)
     if e.ranged then shot(e,h,e.kind=='archer' and 'arrow' or 'magic') end
     if attack(e,h,math.floor((e.damage+rng(-2,2))*GAME.pressure*.93),e.bonus) then
      if (e.kind=='ghoul' or e.kind=='spider') and rng()<.4 then poison(h,6,2);message(h.name..' is poisoned.','#96ac71') end
      if e.kind=='cultist' then h.slow=2.5 end
     end
    end
    if e.boss then
     e.abilityCd=e.abilityCd-dt
     if e.abilityCd<=0 then
      e.abilityCd=e.kind=='king' and 7 or 10
      local radius=e.kind=='oracle' and 2.8 or e.kind=='king' and 3.5 or 3
      local x,z=e.ranged and h.x or e.x,e.ranged and h.z or e.z
      hazard(x,z,radius,e.kind=='king' and 21 or e.kind=='warden' and 18 or 15,e.kind=='oracle' and 'frost' or e.kind=='warden' and 'sunder' or 'grave',1.65)
      message(e.name..' prepares '..(e.kind=='oracle' and 'a shattering spell.' or e.kind=='warden' and 'an iron shockwave.' or 'a gravebound eruption.'),'#c9989d')
     end
     if not e.summoned and (e.kind=='abbot' or e.kind=='king') and e.hp/e.maxhp<.45 then
      e.summoned=true;for i=1,2 do local s=GAME.enemies[i];if s.hp<=0 or not s.active then spawnEnemy('skeleton',i,true) end end
      message(e.name..' calls the fallen to rise again.','#bd9985')
     end
    end
   end
  end
 end end
end
local function updateHazards(dt)
 local r=W.rooms[GAME.room]
 if GAME.phase=='fight' then
  GAME.environmentTimer=GAME.environmentTimer+dt
  if GAME.environmentTimer>8 and (#r.hazards>0 or r.theme=='bridge') then
   GAME.environmentTimer=0;local h=GAME.heroes[rng(1,3)];if h.hp<=0 then h=GAME.leader end
   hazard(h.x,h.z,r.theme=='bridge' and 2 or 2.2,r.theme=='fire' and 16 or 12,r.theme=='spikes' and 'spikes' or r.theme=='bridge' and 'collapse' or r.theme=='fire' and 'fire' or 'poison',1.5)
  end
  for _,h in ipairs(GAME.heroes) do if h.hp>0 then
   for _,p in ipairs(r.hazards) do if (p.kind=='poison' or p.kind=='cave') and (h.x-r.x-p.x)^2+(h.z-r.z-p.z)^2<p.r*p.r then poison(h,4,2) end end
  end end
 elseif GAME.phase=='travel' and not GAME.travelTrap and GAME.phaseTime>GAME.travelHazardAt and GAME.link.danger~='none' then
  GAME.travelTrap=true
  if rng()<.62 then
   local ranger=GAME.heroes[2];local perception=rng(1,20)+(ranger.hp>0 and 4+ranger.gear.boots.tier or 0)
   if perception>=15 then GAME.trapsDetected=GAME.trapsDetected+1;message('Thorne spots danger in the passage.','#aebd89')
   else hazard(GAME.leader.x,GAME.leader.z,2.5,15,GAME.link.danger,1.45);message('The passage springs a '..GAME.link.danger..' trap!','#d4a17f') end
  end
 end
 for i=#GAME.hazards,1,-1 do local t=GAME.hazards[i];t.timer=t.timer+dt
  if t.timer>=t.delay then
   if GAME.render then emit_particles(t.x,.2,t.z,{count=55,shape='ring',color=t.kind=='poison' and '#7ba05d' or t.kind=='frost' and '#8a83b4' or '#d88e53',intensity=1.5,speed=4,life=.6,size=.1}) end
   for _,h in ipairs(GAME.heroes) do if h.hp>0 and dist(h,t)<t.r then
    GAME.hazardHits=GAME.hazardHits+1;hurt(h,t.damage,'#dca074')
    if t.kind=='poison' or t.kind=='web' then poison(h,6,2);h.slow=3 elseif t.kind=='sunder' then h.sunder=12 elseif t.kind=='frost' then h.slow=4 end
   end end
   table.remove(GAME.hazards,i)
  end
 end
end
local function director(dt)
 GAME.phaseTime=GAME.phaseTime+dt
 if living()==0 and not GAME.result then finish('defeat') end
 if GAME.phase=='intro' and GAME.phaseTime>4 then GAME.phase='fight';GAME.phaseTime=0;GAME.environmentTimer=0
 elseif GAME.phase=='fight' and enemiesLeft()==0 then GAME.phase='loot';GAME.phaseTime=0;for _,h in ipairs(GAME.heroes) do h.route=nil end;message('The chamber is clear. Search the reliquary.','#c2b179')
 elseif GAME.phase=='loot' then
  local r=W.rooms[GAME.room];local goal={x=r.x+r.loot[1]-1.6,z=r.z+r.loot[2]+1.3}
  if not GAME.lootGiven and (dist(GAME.leader,goal)<1.1 or GAME.inputSearch and dist(GAME.leader,goal)<3) then
   if r.theme=='stash' and not GAME.ambushed and rng()<.35 then GAME.ambushed=true;spawnEnemy('ghoul',1,true);spawnEnemy('skeleton',1,true);GAME.phase='fight';message('The cache was bait. An ambush!','#cc9789')
   else reward() end
  elseif GAME.lootGiven then
   GAME.lootPause=GAME.lootPause-dt
   if GAME.lootPause<=0 then
    if GAME.room==16 then finish('victory')
    elseif GAME.room==2 or GAME.room==8 then GAME.phase='branch';GAME.phaseTime=0;GAME.branchChoices=GAME.room==2 and {3,4} or {9,10}
    else depart(nextRoom[GAME.room]) end
   end
  end
 elseif GAME.phase=='branch' then
  local choice=GAME.selectedBranch;GAME.selectedBranch=nil
  if GAME.auto and GAME.phaseTime>4 then
   local hp=0;for _,h in ipairs(GAME.heroes) do hp=hp+h.hp/h.maxhp end
   choice=GAME.branchChoices[(GAME.room==2 and (hp<1.9 or living()<3) or rng()<.5) and 1 or 2]
  end
  if choice then depart(choice) end
 elseif GAME.phase=='travel' then
  local p=GAME.link.path[GAME.waypoint]
  if p and dist(GAME.leader,{x=p[1],z=p[2]})<.85 then GAME.waypoint=GAME.waypoint+1 end
  if GAME.waypoint>#GAME.link.path or not GAME.auto and dist(GAME.leader,W.rooms[GAME.nextRoom])<5 then local n=GAME.nextRoom;table.insert(GAME.routeLog,n);activateRoom(n) end
 elseif GAME.result and not GAME.solo and GAME.auto and GAME.phaseTime>12 then
  local old=GAME;makeState(rng(1,10000000),old,old.render)
 end
 GAME.inputSearch=false
end
local function tick(dt)
 if GAME.paused then return end
 GAME.t=GAME.t+dt;GAME.banner=math.max(0,GAME.banner-dt);GAME.potionCd=math.max(0,GAME.potionCd-dt)
 if not GAME.result then
  GAME.elapsed=GAME.elapsed+dt;updateHeroes(dt);updateEnemies(dt);updateHazards(dt);director(dt)
 else director(dt) end
 for i=#GAME.floats,1,-1 do local f=GAME.floats[i];f.life=f.life-dt;f.y=f.y+dt*.6;if f.life<=0 then table.remove(GAME.floats,i) end end
end
local function render(dt)
 if not GAME.render then return end
 for _,h in ipairs(GAME.heroes) do
  local bob=h.hp>0 and (h.moving and math.abs(math.sin(GAME.t*9+h.index))*.05 or math.sin(GAME.t*2+h.index)*.014) or .3
  set_translation(h.id,h.x,bob,h.z);set_rotation(h.id,h.yaw,h.hp>0 and 0 or PI/2,0)
  local stride=h.moving and math.sin(GAME.t*9+h.index)*.4 or 0
  set_rotation(h.left,0,stride,0);set_rotation(h.right,0,-stride,0);set_rotation(h.weapon,0,h.swing>0 and -math.sin(h.swing/.45*PI)*1.3 or 0,0)
 end
 for _,e in ipairs(GAME.enemies) do if e.active then
  set_translation(e.id,e.x,e.hp>0 and (e.moving and math.abs(math.sin(GAME.t*7))*.05 or 0) or .25,e.z)
  set_rotation(e.id,e.yaw,e.hp>0 and 0 or PI/2,0);if e.weapon then set_rotation(e.weapon,0,e.swing>0 and -math.sin(e.swing/.45*PI)*1.4 or 0,0) end
 end end
 for _,s in ipairs(GAME.shots) do if s.active then
  s.life=s.life-dt;local f=1-math.max(0,s.life)/.35;set_translation(s.id,s.ax+(s.bx-s.ax)*f,1.3+math.sin(f*PI)*.2,s.az+(s.bz-s.az)*f);look_at(s.id,s.bx,1.3,s.bz)
  if s.life<=0 then s.active=false;vis(s.id,false) end
 end end
 for i,id in ipairs(fx.rings) do local t=GAME.hazards[i];vis(id,t~=nil)
  if t then set_translation(id,t.x,.18,t.z);set_scale(id,t.r*.74,t.r*.74,.035);set_material(id,{color=t.kind=='poison' and '#7ba05d' or t.kind=='frost' and '#8a83b4' or '#d88e53',emissive=.7,unlit=true}) end
 end
 GAME.lightTimer=GAME.lightTimer+dt
 local leader=GAME.leader;local room=W.rooms[GAME.room]
 if GAME.lightTimer>.3 then
  GAME.lightTimer=0;local closest={}
  for ti,t in ipairs(W.torches) do if not GAME.visibleWorld or GAME.visibleWorld[t.parent] then local d=(t.x-leader.x)^2+(t.z-leader.z)^2;closest[#closest+1]={t=t,d=d,index=ti} end end
  table.sort(closest,function(a,b) return a.d<b.d end)
  GAME.nearTorches=closest
  for i=1,4 do local t=closest[i].t;set_translation(model.lights[i],t.x,2.7,t.z);set_light(model.lights[i],{color=t.color=='#ff5712' and '#eeb071' or '#77b7bf',intensity=40*(1+.065*math.sin(GAME.t*8+i)),range=21}) end
 end
 if GAME.nearTorches then for i=1,4 do local t=GAME.nearTorches[i];local flicker=.96+.065*math.sin(GAME.t*12+i);set_scale(model.torches[t.index],.22*flicker,.42*flicker,.22*flicker) end end
 GAME.fxTimer=GAME.fxTimer+dt
 if GAME.fxTimer>.25 then GAME.fxTimer=0
  emit_particles(leader.x+(math.random()-.5)*12,.6+math.random()*3,leader.z+(math.random()-.5)*12,{count=2,color='#748987',intensity=.15,speed=.05,life=4,size=.025,blend='alpha'})
  for _,t in ipairs(W.torches) do if (t.x-leader.x)^2+(t.z-leader.z)^2<220 then
   emit_particles(t.x,2.2,t.z,{count=1,color=t.color,intensity=1.1,speed=.16,velocity={0,.65,0},life=1.4,size=.025})
  end end
 end
 local targetZoom=GAME.phase=='travel' and 1.08 or GAME.phase=='fight' and 1.12 or math.min(1.5,room.span/25)
 local f=1-math.exp(-dt*2);GAME.camzoom=GAME.camzoom+(targetZoom-GAME.camzoom)*f
 local tx,tz=leader.x,leader.z-1
 GAME.camx=GAME.camx+(tx-GAME.camx)*f;GAME.camz=GAME.camz+(tz-GAME.camz)*f
 local z=GAME.camzoom;local orbit=GAME.orbit and math.sin(GAME.t*.07)*.2 or 0
 set_translation(model.camera,GAME.camx+(12.2+orbit*12)*z,18.2*z,GAME.camz+(15.8-orbit*9)*z);look_at(model.camera,GAME.camx,.45,GAME.camz)
 set_rotation(model.crown,GAME.t*.3,0,.05*math.sin(GAME.t))
end
local function resolveEffects()
 for i,s in ipairs(fx.shots) do s.id=find_entity_by_name('Spell trail '..i) or s.id end
 for i,id in ipairs(fx.rings) do fx.rings[i]=find_entity_by_name('Hazard ring '..i) or id end
 if GAME then GAME.shots=fx.shots;GAME.handlesResolved=true end
end
function on_created(self,previous)
 model={rooms={},corridors={},chests={},lights={},torches={}}
 for _,name in ipairs({'GameCamera','Crown','Knight','Ranger','Mage','KnightWeapon','RangerWeapon','MageWeapon','KnightLegL','KnightLegR','RangerLegL','RangerLegR','MageLegL','MageLegR','KnightSword','KnightHilt','KnightAxe','RangerBow','RangerBowstring','RangerCrossbow','MageStaffcrystal','MageRunes','KnightBody','RangerBody','MageBody'}) do model[name]=find_entity_by_name(name) end
 model.camera=model.GameCamera;model.crown=model.Crown
 for i,t in ipairs(W.torches) do model.torches[i]=find_entity_by_name(t.name) end
 for i=1,19 do model['Enemy'..i]=find_entity_by_name('Enemy'..i);model['Enemy'..i..'Weapon']=find_entity_by_name('Enemy'..i..'Weapon') end
 for i=1,16 do model.rooms[i]=find_entity_by_name('Room'..i);model.chests[i]=find_entity_by_name('Chest'..i) end
 for i=1,#W.links do model.corridors[i]=find_entity_by_name('Corridor'..i) end
 for i=1,4 do model.lights[i]=find_entity_by_name('AtmosphereLight'..(i-1)) end
 fx={shots={},rings={}}
 for i=1,16 do local id=find_entity_by_name('Spell trail '..i) or spawn_primitive('sphere','Spell trail '..i);set_component(id,'Visible',false);set_component(id,'CanCastShadow',false);fx.shots[i]={id=id,active=false} end
 for i=1,8 do local id=find_entity_by_name('Hazard ring '..i) or spawn_primitive('torus','Hazard ring '..i);set_component(id,'Visible',false);set_component(id,'CanCastShadow',false);set_rotation(id,0,PI/2,0);fx.rings[i]=id end
 if previous and previous.version==2 then GAME=previous;resolveEffects();reveal(GAME.phase=='travel' and GAME.nextRoom or nil);return end
 GAME=makeState(math.random(1,10000000),nil,true)
 GAME.ambience=play_sound('content/audio/crypt_drone.wav',{loop=true,volume=.55});GAME.fireAudio=play_sound('content/audio/embers.wav',{loop=true,volume=.45})
end
function update(self,dt)
 if not GAME.handlesResolved then resolveEffects() end
 if is_key_just_pressed('Tab') then GAME.auto=not GAME.auto;GAME.target=nil end
 if is_key_just_pressed('P') then GAME.paused=not GAME.paused end
 if is_key_just_pressed('I') then GAME.inventory=not GAME.inventory end
 if is_key_just_pressed('V') then GAME.map=not GAME.map end
 if is_key_just_pressed('C') then GAME.orbit=not GAME.orbit end
 if is_key_just_pressed('M') then GAME.muted=not GAME.muted;set_master_volume(GAME.muted and 0 or 1) end
 if is_key_just_pressed('R') then local old=GAME;makeState(rng(1,10000000),old,true) end
 for i=1,3 do if is_key_just_pressed(tostring(i)) then GAME.speed=({1,2,4})[i] end end
 if not GAME.auto and not GAME.inventory and not GAME.map and is_mouse_button_just_pressed(0) then local m=get_mouse_position();local p=screen_to_ground(m.x,m.y,0);if p and NAV.walkable(p.x,p.z) then GAME.target={x=p.x,z=p.z};GAME.leader.route=nil end end
 if is_key_just_pressed('F') then GAME.inputSearch=true end
 if is_key_just_pressed('Q') then nova(true) end
 if is_key_just_pressed('E') then potion(true) end
 if GAME.phase=='branch' then if is_key_just_pressed('J') then GAME.selectedBranch=GAME.branchChoices[1] elseif is_key_just_pressed('K') then GAME.selectedBranch=GAME.branchChoices[2] end end
 dt=math.min(dt,.1)*GAME.speed
 local steps=math.max(1,math.ceil(dt/.06));for _=1,steps do tick(dt/steps) end
 if not GAME.paused then render(dt) end
end
function on_ui(self,ui) if GAME then HUD.draw(ui,GAME,W) end end
function on_destroyed(self,reason)
 if reason=='reload' then return GAME end
 for _,s in ipairs(fx.shots) do despawn_entity(s.id) end;for _,id in ipairs(fx.rings) do despawn_entity(id) end
end
RT={
 restart=function(seed) local old=GAME;makeState(seed or rng(1,10000000),old,true) end,
 room=function(n)
  GAME.stop=math.max(0,n-2);activateRoom(n);local r=W.rooms[n]
  for i,h in ipairs(GAME.heroes) do h.x,h.z=NAV.closest(r.x+(i-2)*1.5,r.z+7);h.route=nil end
  GAME.leader=GAME.heroes[1];GAME.camx=r.x;GAME.camz=r.z+4
 end,
 status=function() return {phase=GAME.phase,room=GAME.room,stops=GAME.stop,seed=GAME.seed,pressure=GAME.pressure,hp={GAME.heroes[1].hp,GAME.heroes[2].hp,GAME.heroes[3].hp},mana=GAME.heroes[3].mana,potions=GAME.potions,kills=GAME.kills,upgrades=GAME.upgrades,loot=GAME.lootCount,hazardHits=GAME.hazardHits,detected=GAME.trapsDetected or 0,time=GAME.elapsed,result=GAME.result,finishedAt=GAME.finishedAt,firstFall=GAME.firstFall,victories=GAME.victories,defeats=GAME.defeats,route=GAME.routeLog} end,
 walkable=function() for _,h in ipairs(GAME.heroes) do if h.hp>0 and not NAV.walkable(h.x,h.z) then return false end end return true end,
 skip=function() for _,e in ipairs(GAME.enemies) do if e.active then hurt(e,999,'#a0d3ca') end end end,
 run=function(seed,steps)
  local original=GAME;local g=makeState(seed,nil,false);g.solo=true;g.trapsDetected=0
  for _=1,(steps or 24000) do tick(.05);if g.result then break end end
  local report=RT.status();report.walkable=RT.walkable();report.position={g.leader.x,g.leader.z};report.phaseTime=g.phaseTime
  GAME=original;return report
 end,
 gear=function() local a={};for _,h in ipairs(GAME.heroes) do a[#a+1]={name=h.name,damage=h.damage,ac=h.ac,gear=h.gear} end;return a end
}
